#!/usr/bin/env python3
"""Register org-owned event apps using GitHub's browser manifest flow.

Uses only Python's standard library and gh. Credentials never enter argv, HTML,
or logs. This is an operator tool, not the webhook receiver or developer client.
"""

import argparse
import copy
import html
import http.server
import json
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request


MANIFEST = Path(__file__).resolve().parents[1] / ".github/app-manifest.json"
API_VERSION = "2026-03-10"


class SetupError(Exception):
    pass


def gh_json(account, endpoint):
    """Select a stored account for this child process; never run auth switch."""
    env = dict(os.environ)
    for key in ("GH_TOKEN", "GITHUB_TOKEN", "GH_HOST"):
        env.pop(key, None)
    token = subprocess.run(
        ["gh", "auth", "token", "--hostname", "github.com", "--user", account],
        env=env, capture_output=True, text=True, timeout=30,
    )
    if token.returncode or not token.stdout.strip():
        raise SetupError(f"No stored github.com credential for {account}")
    env["GH_TOKEN"] = token.stdout.strip()
    result = subprocess.run(
        ["gh", "api", "--hostname", "github.com", "-H",
         f"X-GitHub-Api-Version: {API_VERSION}", endpoint],
        env=env, capture_output=True, text=True, timeout=30,
    )
    if result.returncode:
        # Neither gh stderr nor response bodies are safe to echo generically.
        raise SetupError(f"GitHub API request failed for {account}: {endpoint}")
    return json.loads(result.stdout)


def check_owner_access(org, account):
    actor = gh_json(account, "user")
    if actor["login"].casefold() != account.casefold():
        raise SetupError(f"Stored credential identity mismatch for {account}")
    membership = gh_json(account, f"user/memberships/orgs/{org}")
    if membership.get("state") != "active" or membership.get("role") != "admin":
        raise SetupError(f"{account} needs active owner access to {org}")


def private_directory(path):
    path = path.expanduser().absolute()
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid():
        raise SetupError(f"Credential directory must be owned by this user: {path}")
    if stat.S_IMODE(info.st_mode) & 0o077:
        raise SetupError(f"Credential directory must have mode 0700: {path}")
    return path


def write_private(path, value):
    """Exclusive creation prevents accidental overwrite or symlink following."""
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


def exchange_code(code):
    request = urllib.request.Request(
        "https://api.github.com/app-manifests/"
        + urllib.parse.quote(code, safe="") + "/conversions",
        data=b"", method="POST",
        headers={"Accept": "application/vnd.github+json",
                 "X-GitHub-Api-Version": API_VERSION,
                 "User-Agent": "kp-github-app-registration"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    except (urllib.error.URLError, ValueError) as error:
        # The request URL contains a credential: never include the exception.
        raise SetupError("Manifest exchange failed; no credentials were printed.") from None


class Registration:
    def __init__(self, org, account, name, manifest, output):
        self.org, self.account, self.name = org, account, name
        self.manifest = copy.deepcopy(manifest)
        self.manifest["name"] = name
        self.state = secrets.token_urlsafe(32)
        self.started = time.time()
        self.output = output
        self.result = None
        self.receipt = None
        self.error = None
        self.attempted = False
        self.targets = {org: account}
        self.installations = {}

    def preflight(self):
        for org, account in self.targets.items():
            check_owner_access(org, account)

    def complete(self, state, code):
        if not secrets.compare_digest(self.state, state):
            raise SetupError("Registration state did not match.")
        if self.attempted:
            raise SetupError("This registration callback has already been used.")
        if time.time() - self.started > 3600:
            raise SetupError("Registration session expired; restart the setup tool.")
        if not re.fullmatch(r"[A-Za-z0-9_-]{1,256}", code):
            raise SetupError("Invalid registration code.")
        self.attempted = True
        self.result = exchange_code(code)
        # Preserve the one-time response even if validation fails. It must never
        # be lost merely because a browser used the wrong organization.
        write_private(self.output / "credentials.json", self.result)
        owner = self.result.get("owner", {}).get("login", "")
        permissions = self.result.get("permissions", {})
        if owner.casefold() != self.org.casefold():
            raise SetupError("App owner mismatch; credentials saved for recovery, not installed.")
        if permissions != self.manifest["default_permissions"]:
            raise SetupError("App permissions differ from manifest; inspect saved credentials.")
        if set(self.result.get("events", [])) != set(self.manifest["default_events"]):
            raise SetupError("App events differ from manifest; inspect saved credentials.")
        for field in ("id", "slug", "pem", "webhook_secret"):
            if not self.result.get(field):
                raise SetupError("Incomplete app response; inspect saved credentials.")
        if not re.fullmatch(r"[a-zA-Z0-9_-]+", self.result["slug"]):
            raise SetupError("Unexpected app slug; inspect saved credentials.")
        self.receipt = {
            "org": self.org, "api_account": self.account,
            "app_id": self.result["id"], "slug": self.result["slug"],
            "permissions": permissions, "events": self.result["events"],
            "webhook_active_requested": False,
            "registered": True, "installation_verified": False,
            "installation_targets": list(self.targets), "installations": {},
        }
        write_private(self.output / "registration.json", self.receipt)
        print(json.dumps(self.receipt), flush=True)

    def verify_installation(self):
        if not self.receipt:
            raise SetupError(f"{self.org}: register the app first.")
        for org, account in self.targets.items():
            self.verify_target(org, account)

    def verify_target(self, org, account):
        if org in self.installations:
            return
        # A single organization may have more than one page of installed apps.
        for page in range(1, 101):
            result = gh_json(account, f"orgs/{org}/installations?per_page=100&page={page}")
            installations = result.get("installations", [])
            matches = [i for i in installations if i["app_id"] == self.receipt["app_id"]]
            if matches:
                installation = matches[0]
                if installation.get("suspended_at"):
                    raise SetupError(f"{org}: installation is suspended.")
                if installation.get("permissions") != self.manifest["default_permissions"]:
                    raise SetupError(f"{org}: installation permissions differ from manifest.")
                if set(installation.get("events", [])) != set(self.manifest["default_events"]):
                    raise SetupError(f"{org}: installation events differ from manifest.")
                verified = {
                    "org": org, "api_account": account, "app_id": self.receipt["app_id"],
                    "installation_verified": True, "installation_id": installation["id"],
                    "repository_selection": installation["repository_selection"],
                    "permissions": installation["permissions"], "events": installation["events"],
                }
                write_private(self.output / f"installation-{org}.json", verified)
                self.installations[org] = verified
                self.receipt["installations"] = dict(self.installations)
                self.receipt["installation_verified"] = set(self.installations) == set(self.targets)
                print(json.dumps(verified), flush=True)
                return
            if len(installations) < 100:
                break
        raise SetupError(f"{org}: installation not found yet. Complete GitHub's Install step.")


class SetupServer(http.server.HTTPServer):
    def __init__(self, port, registrations, session_path=None):
        super().__init__(("127.0.0.1", port), SetupHandler)
        self.base = f"http://127.0.0.1:{self.server_port}"
        self.session_path = session_path or "/setup/" + secrets.token_urlsafe(32)
        self.csrf_token = secrets.token_urlsafe(32)
        self.registrations = registrations
        for registration in registrations:
            registration.manifest["redirect_url"] = self.base + "/callback/" + registration.org


class SetupHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass  # Callbacks contain one-time credentials.

    def reply(self, status, body):
        data = ("<!doctype html><meta charset='utf-8'><title>GitHub event app setup</title>"
                "<style>body{font:17px system-ui;max-width:850px;margin:3em auto;padding:1em}"
                "button{font:inherit;padding:.5em}pre{overflow:auto}section{margin:2em 0}</style>"
                + body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("X-Frame-Options", "DENY")
        self.send_header("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; "
                         "form-action 'self' https://github.com; frame-ancestors 'none'; base-uri 'none'")
        self.end_headers()
        self.wfile.write(data)

    def valid_host(self):
        return self.headers.get("Host") == urllib.parse.urlsplit(self.server.base).netloc

    def setup_page(self, message=""):
        parts = ["<h1>Developer event app setup</h1>",
                 "<p>Create and install each app below. Webhooks start inactive while the receiver "
                 "is connected. App keys stay in the local operator credential directory.</p>"]
        if message:
            parts.append("<p>" + html.escape(message) + "</p>")
        for registration in self.server.registrations:
            parts.append("<section><h2>" + html.escape(registration.org) + "</h2>")
            if registration.receipt:
                url = "https://github.com/apps/" + registration.receipt["slug"] + "/installations/new"
                parts.append(f'<p>Registered. <a target="_blank" rel="noreferrer" href="{url}">'
                             'Install this app</a> in each organization below.</p>')
                for org, account in registration.targets.items():
                    parts.append("<p>" + html.escape(org) + ": use " + html.escape(account) + ".</p>")
                parts.append("<pre>" + html.escape(json.dumps(registration.receipt, indent=2)) + "</pre>")
            elif registration.error or registration.attempted:
                parts.append("<p>" + html.escape(registration.error or "Registration attempted.") + "</p>")
            else:
                action = (f"https://github.com/organizations/{registration.org}/settings/apps/new?"
                          + urllib.parse.urlencode({"state": registration.state}))
                manifest = html.escape(json.dumps(registration.manifest), quote=True)
                parts.append("<p>In GitHub, sign in as " + html.escape(registration.account)
                             + ". This does not change your CLI account.</p>"
                             + f'<form action="{html.escape(action, quote=True)}" method="post">'
                             + f'<input type="hidden" name="manifest" value="{manifest}">'
                             + '<button type="submit">Create GitHub App</button></form>'
                             + "<details><summary>Review manifest</summary><pre>"
                             + html.escape(json.dumps(registration.manifest, indent=2)) + "</pre></details>")
            parts.append("</section>")
        parts.append(f'<form action="{self.server.session_path}/verify" method="post">'
                     f'<input type="hidden" name="csrf" value="{self.server.csrf_token}">'
                     '<button type="submit">Verify installations</button></form>')
        self.reply(200, "".join(parts))

    def do_GET(self):
        if not self.valid_host():
            self.reply(403, "Invalid host.")
            return
        url = urllib.parse.urlsplit(self.path)
        if url.path in (self.server.session_path, self.server.session_path + "/verify"):
            self.setup_page()
            return
        registrations = [r for r in self.server.registrations if url.path == "/callback/" + r.org]
        if len(registrations) != 1:
            self.reply(404, "Not found.")
            return
        query = urllib.parse.parse_qs(url.query)
        if len(query.get("state", [])) != 1 or len(query.get("code", [])) != 1:
            self.reply(400, "Missing or duplicate callback parameters.")
            return
        registration = registrations[0]
        try:
            registration.complete(query["state"][0], query["code"][0])
        except SetupError as error:
            if registration.attempted:
                registration.error = str(error)
            self.reply(400, html.escape(str(error)))
            return
        except Exception:
            registration.error = "Could not save or validate credentials. Keep this process running for recovery."
            self.reply(500, registration.error)
            return
        # Remove code/state from the visible URL before showing installation links.
        self.send_response(303)
        self.send_header("Location", self.server.session_path)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Referrer-Policy", "no-referrer")
        self.end_headers()

    def do_POST(self):
        # Browsers can send Origin: null with a no-referrer policy. Authenticate
        # the form with its session token; reject any explicit foreign origin.
        if (not self.valid_host() or self.headers.get("Origin") not in (None, "null", self.server.base)
                or self.path != self.server.session_path + "/verify"):
            self.reply(403, "Invalid request.")
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            length = 0
        if (not 0 < length <= 4096
                or self.headers.get_content_type() != "application/x-www-form-urlencoded"):
            self.reply(400, "Invalid form. Reload the setup page before verifying.")
            return
        try:
            form = urllib.parse.parse_qs(self.rfile.read(length).decode("utf-8"))
        except UnicodeDecodeError:
            form = {}
        csrf = form.get("csrf", [])
        if len(csrf) != 1 or not secrets.compare_digest(csrf[0], self.server.csrf_token):
            self.reply(403, "Form expired. Reload the setup page before verifying.")
            return
        messages = []
        for registration in self.server.registrations:
            try:
                registration.verify_installation()
                messages.append(f"{registration.org}: all requested installations verified.")
            except (SetupError, subprocess.TimeoutExpired) as error:
                messages.append(str(error))
        self.setup_page(" ".join(messages))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--org", action="append", required=True, metavar="ORG:GH_ACCOUNT:APP_NAME")
    parser.add_argument("--install-org", action="append", default=[], metavar="ORG:GH_ACCOUNT",
                        help="Also install the single public app in this organization")
    parser.add_argument("--webhook-url", required=True, help="HTTPS receiver URL; starts inactive")
    parser.add_argument("--output-dir", type=Path, required=True, help="Private directory outside any repository")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--public", action="store_true", help="Allow installation outside the owning org")
    parser.add_argument("--resume", action="store_true", help="Resume an existing output directory and URL")
    args = parser.parse_args()
    hook = urllib.parse.urlsplit(args.webhook_url)
    if hook.scheme != "https" or not hook.hostname or hook.username or hook.password or hook.fragment:
        parser.error("--webhook-url must be an HTTPS URL without credentials or fragment")
    manifest = json.loads(MANIFEST.read_text())
    manifest["public"] = args.public
    manifest["hook_attributes"] = {"url": args.webhook_url, "active": False}
    output = private_directory(args.output_dir)
    registrations = []
    for specification in args.org:
        parts = specification.split(":", 2)
        if len(parts) != 3 or not all(re.fullmatch(r"[A-Za-z0-9-]+", p) for p in parts[:2]) or not parts[2]:
            parser.error("--org must be ORG:GH_ACCOUNT:APP_NAME")
        org, account, name = parts
        if any(r.org.casefold() == org.casefold() for r in registrations):
            parser.error("Duplicate organization")
        directory = output / org
        if directory.exists() and not args.resume:
            parser.error(f"Refusing to overwrite existing registration directory: {directory}")
        registration = Registration(org, account, name, manifest, directory)
        registrations.append(registration)
    if args.install_org and (len(registrations) != 1 or not args.public):
        parser.error("--install-org requires exactly one --org and --public")
    for specification in args.install_org:
        parts = specification.split(":")
        if len(parts) != 2 or not all(re.fullmatch(r"[A-Za-z0-9-]+", p) for p in parts):
            parser.error("--install-org must be ORG:GH_ACCOUNT")
        org, account = parts
        if org.casefold() in {o.casefold() for o in registrations[0].targets}:
            parser.error("Duplicate installation target")
        registrations[0].targets[org] = account
    for registration in registrations:
        registration.preflight()
    saved_session = None
    if args.resume:
        saved_session = json.loads((output / "session.json").read_text())
        if not re.fullmatch(r"/setup/[A-Za-z0-9_-]{32,64}", saved_session["session_path"]):
            parser.error("Invalid saved setup URL")
        if set(saved_session["registrations"]) != {r.org for r in registrations}:
            parser.error("Saved session organizations do not match command")
        for registration in registrations:
            private_directory(registration.output)
            saved = saved_session["registrations"][registration.org]
            if saved["targets"] != registration.targets:
                parser.error("Saved installation targets/accounts do not match command")
            registration.state, registration.started = saved["state"], saved["started"]
            receipt_path = registration.output / "registration.json"
            if receipt_path.exists():
                receipt = json.loads(receipt_path.read_text())
                if (receipt["org"] != registration.org
                        or receipt["permissions"] != manifest["default_permissions"]
                        or set(receipt["events"]) != set(manifest["default_events"])):
                    parser.error("Saved registration does not match requested owner/permissions/events")
                registration.receipt, registration.attempted = receipt, True
                for org in registration.targets:
                    path = registration.output / f"installation-{org}.json"
                    if path.exists():
                        installed = json.loads(path.read_text())
                        if installed["app_id"] != receipt["app_id"] or installed["org"] != org:
                            parser.error("Saved installation does not match registration")
                        registration.installations[org] = installed
                receipt["installations"] = dict(registration.installations)
                receipt["installation_verified"] = set(registration.installations) == set(registration.targets)
            elif (registration.output / "credentials.json").exists():
                parser.error("Unvalidated credentials exist; recover the saved app before resuming")
    server = SetupServer(args.port, registrations, saved_session["session_path"] if saved_session else None)
    for registration in registrations:
        if not args.resume:
            private_directory(registration.output)
            write_private(registration.output / "manifest.json", registration.manifest)
    if not args.resume:
        write_private(output / "session.json", {
            "session_path": server.session_path,
            "registrations": {r.org: {"state": r.state, "started": r.started, "targets": r.targets}
                              for r in registrations},
        })
    url = server.base + server.session_path
    print(f"Open {url}", flush=True)
    print(f"Credentials will be saved under {output} (0700 directories, 0600 files).", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    try:
        main()
    except (SetupError, OSError, subprocess.TimeoutExpired):
        # Do not echo arbitrary exceptions, request URLs, tokens, or responses.
        raise SystemExit("Setup failed. Check gh account access, the port, and private output directory permissions.")
