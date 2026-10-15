#!/usr/bin/env python3
"""Install a developer-events profile without distributing App credentials."""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import selectors
import shlex
import signal
import stat
import subprocess
import sys
import time
from typing import Any
import unicodedata
from urllib.parse import urlsplit


PREFIX = "kpghcfg1_"
MAX_CONFIG_BYTES = 64 * 1024
COMMAND_TIMEOUT_SECONDS = 30
SHARED_KEYS = {"version", "endpoint", "audience", "allowed_orgs", "app_slug"}
PROFILE_KEYS = SHARED_KEYS | {"github_account", "google_account"}
LOGIN = re.compile(r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?\Z")


class SetupError(Exception):
    """An error whose message is safe to show without command output."""


def _json(data: bytes | str) -> dict[str, Any]:
    if len(data) > MAX_CONFIG_BYTES:
        raise SetupError("Configuration exceeds the 64 KiB size limit.")

    def unique_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise SetupError("Configuration contains a duplicate field.")
            result[key] = value
        return result

    try:
        result = json.loads(data, object_pairs_hook=unique_pairs,
                            parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
    except (UnicodeError, ValueError, TypeError):
        raise SetupError("Configuration is not valid UTF-8 JSON.") from None
    if not isinstance(result, dict):
        raise SetupError("Configuration must be a JSON object.")
    return result


def _text(value: Any, name: str, maximum: int = 2048) -> str:
    if not isinstance(value, str):
        raise SetupError(f"{name} must be a nonempty string.")
    try:
        size = len(value.encode("utf-8"))
    except UnicodeError:
        raise SetupError(f"{name} must contain valid Unicode.") from None
    if (not value or size > maximum
            or value != value.strip() or any(unicodedata.category(c) == "Cc" for c in value)):
        raise SetupError(f"{name} must be a nonempty string without surrounding whitespace or controls.")
    return value


def _login(value: Any) -> str:
    if not isinstance(value, str) or not LOGIN.fullmatch(value) or "--" in value:
        raise SetupError("github_account must be a GitHub account name.")
    return value


def _google_account(value: Any) -> str:
    value = _text(value, "google_account", 254)
    if "@" not in value or not re.fullmatch(r"[A-Za-z0-9@._+\-]+", value):
        raise SetupError("google_account must be an email address.")
    return value


def validate_config(config: dict[str, Any], *, profile: bool = False,
                    allow_http_loopback: bool = False) -> dict[str, Any]:
    """Strict keys prevent accidentally publishing signing keys or user tokens."""
    if set(config) - (PROFILE_KEYS if profile else SHARED_KEYS):
        raise SetupError("Configuration contains unsupported fields; secrets must never be included.")
    if type(config.get("version")) is not int or config["version"] != 1:
        raise SetupError("Only configuration version 1 is supported.")
    endpoint = _text(config.get("endpoint"), "endpoint")
    if "\\" in endpoint or any(c.isspace() for c in endpoint):
        raise SetupError("endpoint must be an HTTPS URL without credentials, query, or fragment.")
    try:
        parsed = urlsplit(endpoint)
        port = parsed.port
        hostname = parsed.hostname
    except ValueError:
        raise SetupError("endpoint is not a valid URL.") from None
    loopback = hostname in {"localhost", "127.0.0.1", "::1"}
    if (not hostname or parsed.username is not None or parsed.password is not None
            or "?" in endpoint or "#" in endpoint or (port is not None and port == 0)
            or not (parsed.scheme == "https" or
                    (parsed.scheme == "http" and loopback and allow_http_loopback))):
        raise SetupError("endpoint requires HTTPS without credentials, query, or fragment; test HTTP requires explicit loopback opt-in.")
    audience = _text(config.get("audience"), "audience")
    if any(c.isspace() for c in audience):
        raise SetupError("audience must not contain whitespace.")
    orgs = config.get("allowed_orgs")
    if (not isinstance(orgs, list) or not orgs or len(orgs) > 100
            or any(not isinstance(org, str) or not LOGIN.fullmatch(org) or "--" in org for org in orgs)
            or len({org.lower() for org in orgs}) != len(orgs)):
        raise SetupError("allowed_orgs must contain distinct GitHub organization names.")
    result: dict[str, Any] = {
        "version": 1, "endpoint": endpoint.rstrip("/"),
        "audience": audience, "allowed_orgs": list(orgs),
    }
    if "app_slug" in config:
        slug = _text(config["app_slug"], "app_slug", 100)
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*", slug):
            raise SetupError("app_slug must be a GitHub App slug.")
        result["app_slug"] = slug
    if profile:
        result["github_account"] = _login(config.get("github_account"))
        if "google_account" in config:
            result["google_account"] = _google_account(config["google_account"])
    return result


def decode_bundle(bundle: str, *, allow_http_loopback: bool = False) -> dict[str, Any]:
    bundle = bundle.strip()
    if not bundle.startswith(PREFIX) or len(bundle) > MAX_CONFIG_BYTES:
        raise SetupError("Expected a kpghcfg1_ configuration bundle within the size limit.")
    encoded = bundle[len(PREFIX):]
    if not re.fullmatch(r"[A-Za-z0-9_-]+={0,2}", encoded):
        raise SetupError("Configuration bundle is not valid base64url.")
    try:
        payload = base64.b64decode(encoded + "=" * (-len(encoded) % 4), altchars=b"-_", validate=True)
    except (binascii.Error, ValueError):
        raise SetupError("Configuration bundle is not valid base64url.") from None
    return validate_config(_json(payload), allow_http_loopback=allow_http_loopback)


def encode_bundle(config: dict[str, Any], *, allow_http_loopback: bool = False) -> str:
    checked = validate_config(config, allow_http_loopback=allow_http_loopback)
    raw = json.dumps(checked, separators=(",", ":"), sort_keys=True).encode()
    return PREFIX + base64.urlsafe_b64encode(raw).decode().rstrip("=")


def read_source_file(path: str) -> bytes:
    try:
        with Path(path).expanduser().open("rb") as source:
            data = source.read(MAX_CONFIG_BYTES + 1)
    except OSError:
        raise SetupError("Could not read the configuration source file.") from None
    if len(data) > MAX_CONFIG_BYTES:
        raise SetupError("Configuration exceeds the 64 KiB size limit.")
    return data


def _gcs_object(value: str) -> str:
    # Wildcards and generation/query suffixes could read more than the named object.
    if (not re.fullmatch(r"gs://[a-z0-9][a-z0-9._-]*/[^\s*?\[\]#]+", value)
            or any(ord(c) < 32 or ord(c) == 127 for c in value)):
        raise SetupError("--gcs-object must name one gs://bucket/object without wildcards or query suffixes.")
    return value


def fetch_gcs(object_url: str, google_account: str | None) -> bytes:
    command = ["gcloud", "storage", "cat", _gcs_object(object_url)]
    if google_account:
        command.append("--account=" + _google_account(google_account))
    process: subprocess.Popen | None = None
    completed = False
    deadline = time.monotonic() + COMMAND_TIMEOUT_SECONDS
    try:
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                   start_new_session=True, bufsize=0)
        output = bytearray()
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    raise subprocess.TimeoutExpired(command, COMMAND_TIMEOUT_SECONDS)
                chunk = os.read(process.stdout.fileno(), min(8192, MAX_CONFIG_BYTES + 1 - len(output)))
                if not chunk:
                    break
                output.extend(chunk)
                if len(output) > MAX_CONFIG_BYTES:
                    raise SetupError("Configuration exceeds the 64 KiB size limit.")
        returncode = process.wait(timeout=max(0, deadline - time.monotonic()))
        if returncode:
            raise SetupError(f"gcloud configuration fetch failed (exit {returncode}); the existing profile was retained.")
        completed = True
        return bytes(output)
    except subprocess.TimeoutExpired:
        raise SetupError("gcloud configuration fetch timed out; the existing profile was retained.") from None
    except OSError:
        raise SetupError("Could not run gcloud; install it or use --bundle/--config-file.") from None
    finally:
        if process is not None:
            try:
                if not completed:
                    # A separate process group also bounds wrappers or children holding stdout open.
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    try:
                        process.wait(timeout=1)
                    except subprocess.TimeoutExpired:
                        raise SetupError("gcloud did not exit after cancellation; the existing profile was retained.") from None
            finally:
                process.stdout.close()


class PrivateDirectory:
    """Use directory-relative operations; reject symlink traversal and file targets."""

    def __init__(self, path: Path):
        self.path = Path(os.path.abspath(path.expanduser()))
        self.fd: int | None = None

    def __enter__(self) -> "PrivateDirectory":
        flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
        fd = os.open("/", flags)
        try:
            for component in self.path.parts[1:]:
                try:
                    os.mkdir(component, mode=0o700, dir_fd=fd)
                except FileExistsError:
                    pass
                new_fd = os.open(component, flags, dir_fd=fd)
                os.close(fd)
                fd = new_fd
            directory_stat = os.fstat(fd)
            if directory_stat.st_uid != os.getuid():
                raise SetupError("The profile directory must be owned by the current user.")
            if stat.S_IMODE(directory_stat.st_mode) != 0o700:
                raise SetupError("The profile directory must already have mode 0700; choose a dedicated private directory.")
        except BaseException:
            os.close(fd)
            raise
        self.fd = fd
        return self

    def __exit__(self, *_: Any) -> None:
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    def check_target(self, name: str) -> None:
        try:
            info = os.stat(name, dir_fd=self.fd, follow_symlinks=False)
        except FileNotFoundError:
            return
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            raise SetupError("Profile files must be regular files owned by the current user, without symlinks.")

    def read(self, name: str) -> bytes | None:
        self.check_target(name)
        try:
            fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=self.fd)
        except FileNotFoundError:
            return None
        with os.fdopen(fd, "rb") as source:
            info = os.fstat(source.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
                raise SetupError("Profile file is not a regular file owned by the current user.")
            if stat.S_IMODE(info.st_mode) != 0o600:
                raise SetupError("Existing profile files require mode 0600 before reuse.")
            data = source.read(MAX_CONFIG_BYTES + 1)
        if len(data) > MAX_CONFIG_BYTES:
            raise SetupError("Configuration exceeds the 64 KiB size limit.")
        return data

    def write_json(self, name: str, value: dict[str, Any]) -> None:
        self.check_target(name)
        temporary = ".kp-github-" + secrets.token_hex(12)
        try:
            fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                         0o600, dir_fd=self.fd)
            with os.fdopen(fd, "w", encoding="utf-8") as output:
                os.fchmod(output.fileno(), 0o600)
                json.dump(value, output, indent=2, sort_keys=True)
                output.write("\n")
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, name, src_dir_fd=self.fd, dst_dir_fd=self.fd)
            os.fsync(self.fd)
        finally:
            try:
                os.unlink(temporary, dir_fd=self.fd)
            except FileNotFoundError:
                pass


def _source(args: argparse.Namespace) -> tuple[str | None, dict[str, Any] | None]:
    """Identify sources locally before deciding whether a GCS read is necessary."""
    if args.gcs_object:
        identity = "gcs:" + _gcs_object(args.gcs_object)
        config = None
    elif args.bundle:
        config = decode_bundle(args.bundle, allow_http_loopback=args.allow_http_loopback)
        identity = "bundle:" + encode_bundle(config, allow_http_loopback=args.allow_http_loopback)
    elif args.config_file:
        config = validate_config(_json(read_source_file(args.config_file)),
                                 allow_http_loopback=args.allow_http_loopback)
        identity = "file:" + str(Path(args.config_file).expanduser().absolute()) + ":" + json.dumps(config, sort_keys=True)
    else:
        return None, None
    return hashlib.sha256(identity.encode()).hexdigest(), config


def _profile_hash(profile: dict[str, Any]) -> str:
    canonical = json.dumps(profile, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(canonical).hexdigest()


def setup(args: argparse.Namespace) -> tuple[Path, dict[str, Any], bool]:
    path = Path(os.path.abspath(Path(args.output).expanduser()))
    if not path.name or path.name in {".", ".."}:
        raise SetupError("--output must be a profile file path.")
    source_id, supplied = _source(args)
    if args.refresh and source_id is None:
        raise SetupError("--refresh requires --gcs-object, --bundle, or --config-file.")
    if args.github_account is not None:
        _login(args.github_account)
    if args.google_account is not None:
        _google_account(args.google_account)
    sidecar = path.name + ".source.json"
    with PrivateDirectory(path.parent) as directory:
        # Check both destinations before any write or remote fetch.
        directory.check_target(path.name)
        directory.check_target(sidecar)
        cached: dict[str, Any] | None = None
        cache_error: SetupError | None = None
        try:
            raw = directory.read(path.name)
            if raw is not None:
                cached = validate_config(_json(raw), profile=True,
                                         allow_http_loopback=args.allow_http_loopback)
        except SetupError as error:
            cache_error = error
        metadata: dict[str, Any] = {}
        try:
            raw_metadata = directory.read(sidecar)
            if raw_metadata is not None:
                metadata = _json(raw_metadata)
        except SetupError:
            pass  # Missing/invalid metadata cannot establish that an explicit source matches.
        metadata_valid = (
            cached is not None
            and set(metadata) == {"version", "source_sha256", "profile_sha256"}
            and metadata.get("version") == 1
            and isinstance(metadata.get("source_sha256"), str)
            and re.fullmatch(r"[0-9a-f]{64}", metadata["source_sha256"]) is not None
            and metadata.get("profile_sha256") == _profile_hash(cached)
        )
        reuse = cached is not None and not args.refresh and (
            source_id is None or (metadata_valid and metadata["source_sha256"] == source_id))
        effective_source_id = source_id or (metadata["source_sha256"] if metadata_valid else None)
        github_account = args.github_account or (cached or {}).get("github_account")
        if github_account is None:
            raise SetupError("First setup requires --github-account; select an existing gh account.")
        if reuse:
            selected = {key: value for key, value in cached.items() if key in SHARED_KEYS}
        else:
            if source_id is None:
                if cache_error:
                    raise cache_error
                raise SetupError("No cached profile exists; provide --gcs-object, --bundle, or --config-file.")
            selected = supplied
            if selected is None:
                google = args.google_account or (cached or {}).get("google_account")
                raw = fetch_gcs(args.gcs_object, google).strip()
                try:
                    bundle = raw.decode("utf-8")
                except UnicodeError:
                    raise SetupError("GCS object must contain a UTF-8 kpghcfg1_ bundle.") from None
                selected = decode_bundle(bundle, allow_http_loopback=args.allow_http_loopback)
        selected = dict(selected)
        selected["github_account"] = _login(github_account)
        google_account = args.google_account or (cached or {}).get("google_account")
        if google_account is not None:
            selected["google_account"] = _google_account(google_account)
        selected = validate_config(selected, profile=True, allow_http_loopback=args.allow_http_loopback)
        if selected != cached:
            directory.write_json(path.name, selected)
        if effective_source_id is not None:
            next_metadata = {"version": 1, "source_sha256": effective_source_id,
                             "profile_sha256": _profile_hash(selected)}
            if metadata != next_metadata:
                directory.write_json(sidecar, next_metadata)
    return path, selected, reuse


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    install = commands.add_parser("setup", help="Save or reuse a private local developer profile")
    sources = install.add_mutually_exclusive_group()
    sources.add_argument("--gcs-object", help="Exact operator-provided gs://bucket/object containing a bundle")
    sources.add_argument("--bundle", help="Pasted kpghcfg1_ bundle (contains configuration, never secrets)")
    sources.add_argument("--config-file", help="JSON shared configuration file")
    install.add_argument("--github-account", help="Existing gh account to select without changing gh defaults")
    install.add_argument("--google-account", help="Google account for this profile and optional GCS read")
    install.add_argument("--output", default="~/.config/kp-github/events.json")
    install.add_argument("--refresh", action="store_true", help="Explicitly read the selected source again")
    install.add_argument("--allow-http-loopback", action="store_true", help="Permit HTTP on localhost for tests only")
    pack = commands.add_parser("pack", help="Pack operator configuration for GCS or copy/paste distribution")
    pack.add_argument("--config-file", required=True)
    pack.add_argument("--allow-http-loopback", action="store_true", help="Permit HTTP on localhost for tests only")
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "pack":
            print(encode_bundle(_json(read_source_file(args.config_file)),
                                allow_http_loopback=args.allow_http_loopback))
        else:
            path, profile, reused = setup(args)
            print("Reused cached developer configuration." if reused else "Saved developer configuration.")
            print("Launch with your selected account:")
            print("KP_GITHUB_ACCOUNT=" + shlex.quote(profile["github_account"]) +
                  " KP_GITHUB_EVENTS_CONFIG=" + shlex.quote(str(path)) + " kp-github-mcp")
            print("The receiver verifies access when you connect; this profile does not grant access.")
        return 0
    except SetupError as error:
        print(f"Setup failed: {error}", file=sys.stderr)
        return 1
    except OSError:
        print("Setup failed: could not access the private profile directory or files (symlinks are not allowed).", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
