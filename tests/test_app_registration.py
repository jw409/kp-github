"""Local regression tests for the credential-bearing registration boundary."""

import contextlib
import http.client
import importlib.util
import io
import json
import os
import re
from pathlib import Path
import stat
import tempfile
import threading
import unittest
from unittest.mock import patch
from urllib.parse import urlencode


spec = importlib.util.spec_from_file_location(
    "register_github_app", Path(__file__).resolve().parents[1] / "scripts/register_github_app.py"
)
app = importlib.util.module_from_spec(spec)
spec.loader.exec_module(app)


class RegistrationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.output = app.private_directory(Path(self.temp.name) / "private")
        self.manifest = json.loads(app.MANIFEST.read_text())
        self.registration = app.Registration("Example", "alice", "Example Events", self.manifest, self.output)
        self.response = {
            "id": 42, "slug": "example-events", "owner": {"login": "Example"},
            "permissions": self.manifest["default_permissions"],
            "events": self.manifest["default_events"],
            "pem": "TEST-PRIVATE-KEY", "webhook_secret": "TEST-WEBHOOK-SECRET",
            "client_secret": "TEST-CLIENT-SECRET",
        }

    def complete(self):
        with patch.object(app, "exchange_code", return_value=self.response), contextlib.redirect_stdout(io.StringIO()):
            self.registration.complete(self.registration.state, "temporary-code")

    def test_callback_checks_state_and_rejects_replay_without_exchange(self):
        with patch.object(app, "exchange_code", return_value=self.response) as exchange:
            with self.assertRaises(app.SetupError):
                self.registration.complete("wrong-state", "temporary-code")
            exchange.assert_not_called()
            with contextlib.redirect_stdout(io.StringIO()):
                self.registration.complete(self.registration.state, "temporary-code")
            with self.assertRaises(app.SetupError):
                self.registration.complete(self.registration.state, "temporary-code")
            self.assertEqual(exchange.call_count, 1)

    def test_secrets_stay_in_private_file_not_receipts_or_output(self):
        stream = io.StringIO()
        with patch.object(app, "exchange_code", return_value=self.response), contextlib.redirect_stdout(stream):
            self.registration.complete(self.registration.state, "temporary-code")
        credentials = self.output / "credentials.json"
        receipt = self.output / "registration.json"
        self.assertEqual(stat.S_IMODE(credentials.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.output.stat().st_mode), 0o700)
        self.assertEqual(json.loads(credentials.read_text()), self.response)
        for value in ("TEST-PRIVATE-KEY", "TEST-WEBHOOK-SECRET", "TEST-CLIENT-SECRET"):
            self.assertNotIn(value, receipt.read_text() + stream.getvalue())

    def test_wrong_owner_preserves_one_time_credentials_but_cannot_install(self):
        self.response["owner"]["login"] = "WrongOrg"
        with self.assertRaises(app.SetupError):
            self.complete()
        self.assertTrue((self.output / "credentials.json").exists())
        self.assertIsNone(self.registration.receipt)

    def test_excess_permission_is_not_accepted(self):
        self.response["permissions"] = dict(self.response["permissions"], contents="write")
        with self.assertRaises(app.SetupError):
            self.complete()
        self.assertIsNone(self.registration.receipt)

    def test_no_overwrite_or_symlink_following(self):
        target = self.output / "target"
        target.write_text("keep")
        link = self.output / "credentials.json"
        link.symlink_to(target)
        with self.assertRaises(FileExistsError):
            app.write_private(link, self.response)
        self.assertEqual(target.read_text(), "keep")

    def test_refuses_shared_directory(self):
        self.output.chmod(0o755)
        with self.assertRaises(app.SetupError):
            app.private_directory(self.output)

    def test_account_selected_only_in_child_environment(self):
        calls = []

        def run(argv, **kwargs):
            calls.append((argv, dict(kwargs["env"])))
            if argv[1:3] == ["auth", "token"]:
                return app.subprocess.CompletedProcess(argv, 0, "selected-token\n", "")
            return app.subprocess.CompletedProcess(argv, 0, '{"login":"alice"}', "")

        with patch.dict(os.environ, {"GH_TOKEN": "ambient-gh", "GITHUB_TOKEN": "ambient-github"}, clear=True), patch.object(app.subprocess, "run", side_effect=run):
            self.assertEqual(app.gh_json("alice", "user"), {"login": "alice"})
            self.assertEqual(os.environ["GH_TOKEN"], "ambient-gh")
            self.assertEqual(os.environ["GITHUB_TOKEN"], "ambient-github")
        self.assertFalse("GH_TOKEN" in calls[0][1])
        self.assertEqual(calls[1][1]["GH_TOKEN"], "selected-token")
        self.assertFalse("GITHUB_TOKEN" in calls[1][1])
        self.assertTrue(all("switch" not in argv and "selected-token" not in argv for argv, _ in calls))

    def test_installation_must_match_app_and_expected_permissions(self):
        self.complete()
        wrong = {"app_id": 7, "id": 1}
        correct = {"app_id": 42, "id": 99, "repository_selection": "selected",
                   "permissions": self.manifest["default_permissions"],
                   "events": self.manifest["default_events"], "suspended_at": None}
        with patch.object(app, "gh_json", return_value={"installations": [wrong]}):
            with self.assertRaises(app.SetupError):
                self.registration.verify_installation()
        self.assertFalse(self.registration.receipt["installation_verified"])
        with patch.object(app, "gh_json", return_value={"installations": [wrong, correct]}), contextlib.redirect_stdout(io.StringIO()):
            self.registration.verify_installation()
        self.assertEqual(self.registration.receipt["installations"]["Example"]["installation_id"], 99)
        self.assertTrue((self.output / "installation-Example.json").exists())

    def test_shared_app_verifies_both_orgs_with_their_own_accounts(self):
        self.registration.targets["SecondOrg"] = "bob"
        self.complete()
        installation = {"app_id": 42, "id": 99, "repository_selection": "all",
                        "permissions": self.manifest["default_permissions"],
                        "events": self.manifest["default_events"], "suspended_at": None}
        with patch.object(app, "gh_json", side_effect=[{"installations": [installation]}, {"installations": []}]):
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(app.SetupError):
                self.registration.verify_installation()
        self.assertFalse(self.registration.receipt["installation_verified"])
        with patch.object(app, "gh_json", return_value={"installations": [dict(installation, id=100)]}) as api:
            with contextlib.redirect_stdout(io.StringIO()):
                self.registration.verify_installation()
            api.assert_called_once_with("bob", "orgs/SecondOrg/installations?per_page=100&page=1")
        self.assertTrue(self.registration.receipt["installation_verified"])
        self.assertEqual(self.registration.installations["SecondOrg"]["installation_id"], 100)

    def test_http_callback_has_no_secret_body_and_wrong_host_is_rejected(self):
        server = app.SetupServer(0, [self.registration])
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(thread.join, 2)
        self.addCleanup(server.shutdown)
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        self.addCleanup(connection.close)
        connection.request("GET", server.session_path, headers={"Host": "attacker.example"})
        response = connection.getresponse()
        self.assertEqual(response.status, 403)
        response.read()
        query = urlencode({"state": self.registration.state, "code": "temporary-code"})
        with patch.object(app, "exchange_code", return_value=self.response), contextlib.redirect_stdout(io.StringIO()):
            connection.request("GET", "/callback/Example?" + query)
            response = connection.getresponse()
            self.assertEqual(response.status, 303)
            self.assertEqual(response.getheader("Location"), server.session_path)
            self.assertEqual(response.read(), b"")
        connection.request("GET", server.session_path)
        response = connection.getresponse()
        body = response.read().decode()
        self.assertIn("installations/new", body)
        self.assertNotIn("TEST-PRIVATE-KEY", body)
        self.assertNotIn("TEST-WEBHOOK-SECRET", body)
        self.assertNotIn("temporary-code", body)

    def test_verify_form_works_with_browser_null_origin_and_rejects_forgery(self):
        server = app.SetupServer(0, [self.registration])
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(thread.join, 2)
        self.addCleanup(server.shutdown)
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port)
        self.addCleanup(connection.close)
        connection.request("GET", server.session_path)
        response = connection.getresponse()
        page = response.read().decode()
        match = re.search(r'name="csrf" value="([A-Za-z0-9_-]+)"', page)
        self.assertIsNotNone(match, "Verification form must include a CSRF token")
        token = match.group(1)
        cases = [("null", token, 200), (None, token, 200),
                 (server.base, token, 200), ("https://attacker.example", token, 403),
                 ("null", "wrong-token", 403)]
        with patch.object(self.registration, "verify_installation") as verify:
            for origin, csrf, expected in cases:
                headers = {"Content-Type": "application/x-www-form-urlencoded"}
                if origin is not None:
                    headers["Origin"] = origin
                connection.request("POST", server.session_path + "/verify", urlencode({"csrf": csrf}), headers)
                response = connection.getresponse()
                self.assertEqual(response.status, expected)
                response.read()
            self.assertEqual(verify.call_count, 3)
        connection.request("GET", server.session_path + "/verify")
        response = connection.getresponse()
        self.assertEqual(response.status, 200)
        self.assertIn("Developer event app setup", response.read().decode())


if __name__ == "__main__":
    unittest.main()
