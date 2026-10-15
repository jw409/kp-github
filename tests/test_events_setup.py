"""Offline behavioral coverage for the developer-facing bootstrap CLI."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import time
import tempfile
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location(
    "github_events_setup", Path(__file__).resolve().parents[1] / "scripts/github_events_setup.py")
setup = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(setup)


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.profile = self.directory / "private" / "events.json"
        self.config = {
            "version": 1,
            "endpoint": "https://events.example.test/v1",
            "audience": "https://events.example.test",
            "allowed_orgs": ["Meshlyai", "Meshly-Open-Source"],
            "app_slug": "meshly-open-source-dev-events",
        }
        self.bundle = setup.encode_bundle(self.config)

    def invoke(self, *arguments):
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = setup.main(["setup", "--output", str(self.profile), *arguments])
        return code, stdout.getvalue(), stderr.getvalue()

    def initial(self):
        code, _, error = self.invoke("--bundle", self.bundle, "--github-account", "jw408")
        self.assertEqual(code, 0, error)

    def read_profile(self):
        return json.loads(self.profile.read_text())

    def fake_gcloud(self, program):
        """Exercise pipe/deadline behavior with real, disposable Python children."""
        original_popen = subprocess.Popen
        processes = []

        def start(command, **kwargs):
            self.assertEqual(command[:3], ["gcloud", "storage", "cat"])
            child = original_popen([sys.executable, "-c", program], **kwargs)
            processes.append(child)
            return child

        return start, processes

    def test_fetch_uses_bounded_argument_array_and_selected_account(self):
        start, processes = self.fake_gcloud("import sys; sys.stdout.write(" + repr(self.bundle) + ")")
        with mock.patch.object(setup.subprocess, "Popen", side_effect=start) as popen:
            result = setup.fetch_gcs("gs://sample-configs/one", "dev@example.test")
        self.assertEqual(result, self.bundle.encode())
        self.assertEqual(popen.call_args.args[0], ["gcloud", "storage", "cat", "gs://sample-configs/one", "--account=dev@example.test"])
        self.assertNotIn("shell", popen.call_args.kwargs)
        self.assertTrue(popen.call_args.kwargs["start_new_session"])
        self.assertEqual(processes[0].returncode, 0)

    def test_real_child_timeout_kills_and_reaps_without_clobbering_cache(self):
        self.initial()
        before = self.profile.read_bytes()
        start, processes = self.fake_gcloud("import time; time.sleep(60)")
        started = time.monotonic()
        with mock.patch.object(setup.subprocess, "Popen", side_effect=start), \
                mock.patch.object(setup, "COMMAND_TIMEOUT_SECONDS", 0.1):
            code, _, error = self.invoke("--gcs-object", "gs://sample-configs/one", "--refresh")
        self.assertEqual(code, 1)
        self.assertIn("timed out", error)
        self.assertLess(time.monotonic() - started, 3)
        self.assertIsNotNone(processes[0].returncode)
        self.assertLess(processes[0].returncode, 0)
        self.assertEqual(self.profile.read_bytes(), before)

    def test_real_child_stderr_is_not_exposed(self):
        self.initial()
        before = self.profile.read_bytes()
        start, _ = self.fake_gcloud("import sys; sys.stderr.write('synthetic-sensitive-diagnostic'); sys.exit(17)")
        with mock.patch.object(setup.subprocess, "Popen", side_effect=start):
            code, output, error = self.invoke("--gcs-object", "gs://sample-configs/one", "--refresh")
        self.assertEqual(code, 1)
        self.assertIn("exit 17", error)
        self.assertNotIn("synthetic-sensitive-diagnostic", output + error)
        self.assertEqual(self.profile.read_bytes(), before)

    def test_pasted_bundle_creates_private_profile_and_safe_launch_command(self):
        with mock.patch.object(setup.subprocess, "Popen", side_effect=AssertionError("no network")):
            code, output, error = self.invoke("--bundle", self.bundle, "--github-account", "jw408",
                                             "--google-account", "developer@example.test")
        self.assertEqual(code, 0, error)
        self.assertEqual(self.read_profile(), {**self.config, "github_account": "jw408",
                                              "google_account": "developer@example.test"})
        self.assertEqual(stat.S_IMODE(self.profile.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.profile.parent.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(Path(str(self.profile) + ".source.json").stat().st_mode), 0o600)
        self.assertIn("KP_GITHUB_ACCOUNT=jw408", output)
        self.assertIn("KP_GITHUB_EVENTS_CONFIG=", output)
        self.assertNotIn(self.bundle, output)
        self.assertIn("does not grant access", output)

    def test_cached_profile_without_source_does_not_revalidate_or_rewrite(self):
        self.initial()
        before = self.profile.stat().st_mtime_ns
        with mock.patch.object(setup.subprocess, "Popen", side_effect=AssertionError("no network")):
            code, output, error = self.invoke()
        self.assertEqual(code, 0, error)
        self.assertIn("Reused cached", output)
        self.assertEqual(self.profile.stat().st_mtime_ns, before)

    def test_unchanged_gcs_source_fetches_once_then_uses_cache(self):
        command_result = self.bundle.encode()
        with mock.patch.object(setup, "fetch_gcs", return_value=command_result) as run:
            code, _, error = self.invoke("--gcs-object", "gs://sample-configs/everyone/github-events",
                                         "--github-account", "jw408", "--google-account", "dev@example.test")
            self.assertEqual(code, 0, error)
            first_call = run.call_args
            code, output, error = self.invoke("--gcs-object", "gs://sample-configs/everyone/github-events")
            self.assertEqual(code, 0, error)
            run.assert_called_once()
        self.assertIn("Reused cached", output)
        self.assertEqual(first_call.args, ("gs://sample-configs/everyone/github-events", "dev@example.test"))

    def test_refresh_reads_gcs_and_updates_config(self):
        object_url = "gs://sample-configs/everyone/github-events"
        changed = {**self.config, "audience": "new-audience"}
        responses = [setup.encode_bundle(c).encode()
                     for c in (self.config, changed)]
        with mock.patch.object(setup, "fetch_gcs", side_effect=responses) as run:
            self.assertEqual(self.invoke("--gcs-object", object_url, "--github-account", "jw408")[0], 0)
            code, _, error = self.invoke("--gcs-object", object_url, "--refresh")
            self.assertEqual(code, 0, error)
            self.assertEqual(run.call_count, 2)
        self.assertEqual(self.read_profile()["audience"], "new-audience")

    def test_source_change_reads_new_gcs_object(self):
        with mock.patch.object(setup, "fetch_gcs",
                               return_value=self.bundle.encode()) as run:
            self.assertEqual(self.invoke("--gcs-object", "gs://sample-configs/one", "--github-account", "jw408")[0], 0)
            self.assertEqual(self.invoke("--gcs-object", "gs://sample-configs/two")[0], 0)
            self.assertEqual(run.call_count, 2)

    def test_torn_profile_and_metadata_write_cannot_reuse_wrong_source(self):
        source_a = "gs://sample-configs/a"
        source_b = "gs://sample-configs/b"
        config_b = {**self.config, "audience": "audience-b"}
        responses = [setup.encode_bundle(c).encode()
                     for c in (self.config, config_b, self.config)]
        real_write = setup.PrivateDirectory.write_json

        def fail_sidecar(directory, name, value):
            if name.endswith(".source.json"):
                raise OSError("synthetic sidecar failure")
            return real_write(directory, name, value)

        with mock.patch.object(setup, "fetch_gcs", side_effect=responses) as run:
            self.assertEqual(self.invoke("--gcs-object", source_a, "--github-account", "jw408")[0], 0)
            with mock.patch.object(setup.PrivateDirectory, "write_json", new=fail_sidecar):
                self.assertEqual(self.invoke("--gcs-object", source_b)[0], 1)
            self.assertEqual(self.read_profile()["audience"], "audience-b")
            code, output, error = self.invoke("--gcs-object", source_a)
            self.assertEqual(code, 0, error)
            self.assertEqual(run.call_count, 3)
        self.assertNotIn("Reused cached", output)
        self.assertEqual(self.read_profile()["audience"], self.config["audience"])

    def test_account_change_updates_profile_hash_without_refetch(self):
        source = "gs://sample-configs/one"
        with mock.patch.object(setup, "fetch_gcs",
                               return_value=self.bundle.encode()) as run:
            self.assertEqual(self.invoke("--gcs-object", source, "--github-account", "jw408")[0], 0)
            self.assertEqual(self.invoke("--github-account", "jw409")[0], 0)
            self.assertEqual(self.invoke("--gcs-object", source)[0], 0)
            run.assert_called_once()
        metadata = json.loads(Path(str(self.profile) + ".source.json").read_text())
        self.assertEqual(metadata["profile_sha256"], setup._profile_hash(self.read_profile()))

    def test_refresh_auth_failure_does_not_clobber_cache_or_leak_stderr(self):
        self.initial()
        before = self.profile.read_bytes()
        metadata_path = Path(str(self.profile) + ".source.json")
        metadata_before = metadata_path.read_bytes()
        with mock.patch.object(setup, "fetch_gcs",
                               side_effect=setup.SetupError("gcloud configuration fetch failed; the existing profile was retained.")):
            code, output, error = self.invoke("--gcs-object", "gs://sample-configs/one", "--refresh")
        self.assertEqual(code, 1)
        self.assertIn("existing profile was retained", error)
        self.assertNotIn("secret-diagnostic", output + error)
        self.assertEqual(self.profile.read_bytes(), before)
        self.assertEqual(metadata_path.read_bytes(), metadata_before)

    def test_invalid_gcs_refresh_preserves_profile(self):
        self.initial()
        before = self.profile.read_bytes()
        with mock.patch.object(setup, "fetch_gcs",
                               return_value=b"not-a-bundle"):
            code, _, error = self.invoke("--gcs-object", "gs://sample-configs/one", "--refresh")
        self.assertEqual(code, 1)
        self.assertIn("Expected a kpghcfg1_", error)
        self.assertEqual(self.profile.read_bytes(), before)

    def test_timeout_preserves_profile_and_has_safe_error(self):
        self.initial()
        before = self.profile.read_bytes()
        with mock.patch.object(setup, "fetch_gcs",
                               side_effect=setup.SetupError("gcloud configuration fetch timed out; the existing profile was retained.")):
            code, output, error = self.invoke("--gcs-object", "gs://sample-configs/one", "--refresh")
        self.assertEqual(code, 1)
        self.assertNotIn("sensitive", output + error)
        self.assertEqual(self.profile.read_bytes(), before)

    def test_config_file_content_change_invalidates_cache_without_network(self):
        source = self.directory / "shared.json"
        source.write_text(json.dumps(self.config))
        with mock.patch.object(setup.subprocess, "Popen", side_effect=AssertionError("no network")):
            self.assertEqual(self.invoke("--config-file", str(source), "--github-account", "jw408")[0], 0)
            source.write_text(json.dumps({**self.config, "audience": "changed"}))
            self.assertEqual(self.invoke("--config-file", str(source))[0], 0)
        self.assertEqual(self.read_profile()["audience"], "changed")

    def test_account_override_reuses_cache_without_mutating_machine_defaults(self):
        self.initial()
        synthetic = {"GH_TOKEN": "synthetic-only", "GITHUB_TOKEN": "synthetic-other"}
        with mock.patch.dict(os.environ, synthetic, clear=True), \
                mock.patch.object(setup.subprocess, "Popen", side_effect=AssertionError("no subprocess")):
            code, _, error = self.invoke("--github-account", "jw409", "--google-account", "another@example.test")
            self.assertEqual(dict(os.environ), synthetic)
        self.assertEqual(code, 0, error)
        self.assertEqual(self.read_profile()["github_account"], "jw409")

    def test_pack_roundtrip_prints_only_bundle(self):
        source = self.directory / "shared.json"
        source.write_text(json.dumps(self.config))
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(setup.main(["pack", "--config-file", str(source)]), 0)
        self.assertEqual(setup.decode_bundle(output.getvalue()), self.config)

    def test_pack_rejects_secrets_without_echoing_names_or_values(self):
        for name in ("private_key", "webhook_secret", "token", "client_secret", "pem"):
            with self.subTest(name=name):
                source = self.directory / "shared.json"
                source.write_text(json.dumps({**self.config, name: "synthetic-secret-value"}))
                output, error = io.StringIO(), io.StringIO()
                with contextlib.redirect_stdout(output), contextlib.redirect_stderr(error):
                    self.assertEqual(setup.main(["pack", "--config-file", str(source)]), 1)
                self.assertEqual(output.getvalue(), "")
                self.assertNotIn("synthetic-secret-value", error.getvalue())
                self.assertNotIn(name, error.getvalue())

    def test_invalid_endpoint_is_rejected(self):
        for endpoint in ("http://events.example.test", "http://127.0.0.1:9000",
                         "https://user:secret@example.test", "https://example.test?q=secret",
                         "https://example.test#secret", "https://example.test?", "https://example.test#",
                         "https:///missing", "https://example.test:bad", "https://example.test:0",
                         "https://example.test/path with space", "https://example.test\\path"):
            with self.subTest(endpoint=endpoint), self.assertRaises(setup.SetupError):
                setup.validate_config({**self.config, "endpoint": endpoint})

    def test_http_loopback_requires_explicit_opt_in_even_for_cache(self):
        config = {**self.config, "endpoint": "http://127.0.0.1:9000"}
        bundle = setup.encode_bundle(config, allow_http_loopback=True)
        self.assertEqual(self.invoke("--bundle", bundle, "--github-account", "jw408")[0], 1)
        self.assertEqual(self.invoke("--bundle", bundle, "--github-account", "jw408", "--allow-http-loopback")[0], 0)
        self.assertEqual(self.invoke()[0], 1)
        self.assertEqual(self.invoke("--allow-http-loopback")[0], 0)
        with self.assertRaises(setup.SetupError):
            setup.validate_config({**config, "endpoint": "http://example.test"}, allow_http_loopback=True)

    def test_malformed_schema_rejected(self):
        changes = ({"version": True}, {"version": 2}, {"allowed_orgs": []},
                   {"allowed_orgs": ["Meshlyai", "meshlyai"]}, {"allowed_orgs": [None]},
                   {"allowed_orgs": ["../../wrong"]}, {"allowed_orgs": [f"org-{i}" for i in range(101)]},
                   {"audience": ""}, {"audience": "contains space"}, {"app_slug": "a/b"})
        for change in changes:
            with self.subTest(change=change), self.assertRaises(setup.SetupError):
                setup.validate_config({**self.config, **change})
        for payload in (b'{}', b'[]', b'{"version": 1, "version": 1}', b'{"version": NaN}'):
            with self.subTest(payload=payload), self.assertRaises(setup.SetupError):
                setup.validate_config(setup._json(payload))

    def test_field_lengths_are_utf8_bytes_and_unicode_controls_rejected(self):
        for change in ({"audience": "é" * 1025}, {"endpoint": "https://example.test/" + "é" * 1025},
                       {"audience": "audience\u0090"}, {"audience": "\ud800"}):
            with self.subTest(change=change), self.assertRaises(setup.SetupError):
                setup.validate_config({**self.config, **change})

    def test_profile_symlink_cannot_overwrite_target(self):
        self.profile.parent.mkdir(mode=0o700)
        victim = self.directory / "victim.json"
        victim.write_text("leave unchanged")
        self.profile.symlink_to(victim)
        code, _, error = self.invoke("--bundle", self.bundle, "--github-account", "jw408")
        self.assertEqual(code, 1)
        self.assertIn("symlinks", error)
        self.assertEqual(victim.read_text(), "leave unchanged")

    def test_directory_symlink_rejected(self):
        other = self.directory / "other"
        other.mkdir(mode=0o700)
        self.profile.parent.symlink_to(other, target_is_directory=True)
        code, _, error = self.invoke("--bundle", self.bundle, "--github-account", "jw408")
        self.assertEqual(code, 1)
        self.assertIn("symlinks", error)
        self.assertFalse((other / self.profile.name).exists())

    def test_existing_shared_directory_permissions_are_not_changed(self):
        self.profile.parent.mkdir(mode=0o755)
        self.profile.parent.chmod(0o755)
        code, _, error = self.invoke("--bundle", self.bundle, "--github-account", "jw408")
        self.assertEqual(code, 1)
        self.assertIn("dedicated private directory", error)
        self.assertEqual(stat.S_IMODE(self.profile.parent.stat().st_mode), 0o755)
        self.assertFalse(self.profile.exists())

    def test_first_setup_without_account_does_not_fetch(self):
        with mock.patch.object(setup.subprocess, "Popen") as run:
            code, _, error = self.invoke("--gcs-object", "gs://sample-configs/one")
        self.assertEqual(code, 1)
        self.assertIn("--github-account", error)
        run.assert_not_called()

    def test_sidecar_symlink_does_not_allow_partial_profile_write(self):
        self.initial()
        before = self.profile.read_bytes()
        sidecar = Path(str(self.profile) + ".source.json")
        sidecar.unlink()
        sidecar.symlink_to(self.directory / "absent")
        code, _, _ = self.invoke("--github-account", "jw409")
        self.assertEqual(code, 1)
        self.assertEqual(self.profile.read_bytes(), before)

    def test_explicit_refresh_requires_source(self):
        self.initial()
        code, _, error = self.invoke("--refresh")
        self.assertEqual(code, 1)
        self.assertIn("requires", error)

    def test_unsafe_gcs_object_rejected_without_command(self):
        for url in ("gs://sample-configs/*", "gs://sample-configs/file?x", "gs://sample-configs/file#1",
                    "gs://sample-configs/", "https://sample-configs/file"):
            with self.subTest(url=url), mock.patch.object(setup.subprocess, "Popen") as run:
                self.assertEqual(self.invoke("--gcs-object", url, "--github-account", "jw408")[0], 1)
                run.assert_not_called()

    def test_failed_atomic_replace_retains_existing_profile(self):
        self.initial()
        before = self.profile.read_bytes()
        with mock.patch.object(setup.os, "replace", side_effect=OSError("synthetic failure")):
            code, _, _ = self.invoke("--github-account", "jw409")
        self.assertEqual(code, 1)
        self.assertEqual(self.profile.read_bytes(), before)
        self.assertEqual(list(self.profile.parent.glob(".kp-github-*")), [])

    def test_oversized_gcs_result_is_rejected_without_profile(self):
        start, processes = self.fake_gcloud("import os, time; os.write(1, b'x' * 65537); time.sleep(60)")
        started = time.monotonic()
        with mock.patch.object(setup.subprocess, "Popen", side_effect=start):
            code, _, error = self.invoke("--gcs-object", "gs://sample-configs/one", "--github-account", "jw408")
        self.assertEqual(code, 1)
        self.assertIn("size limit", error)
        self.assertLess(time.monotonic() - started, 3)
        self.assertIsNotNone(processes[0].returncode)
        self.assertLess(processes[0].returncode, 0)
        self.assertFalse(self.profile.exists())


if __name__ == "__main__":
    unittest.main()
