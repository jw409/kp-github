"""Keep published sample object bytes compatible with the real bootstrap CLI."""

import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import unittest
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
SAMPLES = ROOT / "examples/github-events"
SCRIPT = ROOT / "scripts/github_events_setup.py"
SPEC = importlib.util.spec_from_file_location("sample_bootstrap", SCRIPT)
bootstrap = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bootstrap)


class ConfigSamplesTests(unittest.TestCase):
    def test_published_bundle_bytes_match_real_pack_command(self):
        for name in ("meshly", "selfhost"):
            with self.subTest(sample=name):
                source = SAMPLES / f"{name}.sample.json"
                bundle = (SAMPLES / f"{name}.sample.bundle").read_bytes()
                packed = subprocess.run([sys.executable, str(SCRIPT), "pack", "--config-file", str(source)],
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                        check=True, timeout=5)
                self.assertEqual(bundle, packed.stdout)
                self.assertTrue(bundle.startswith(b"kpghcfg1_"))
                self.assertEqual(bundle.count(b"\n"), 1)
                self.assertTrue(bundle.endswith(b"\n"))
                self.assertNotIn(b"\r", bundle)
                self.assertNotIn(b"=", bundle)
                self.assertLessEqual(len(bundle), bootstrap.MAX_CONFIG_BYTES)
                self.assertEqual(bootstrap.decode_bundle(bundle.decode("ascii")),
                                 bootstrap.validate_config(json.loads(source.read_text())))

    def test_samples_cannot_point_at_a_production_receiver(self):
        for name in ("meshly", "selfhost"):
            with self.subTest(sample=name):
                config = json.loads((SAMPLES / f"{name}.sample.json").read_text())
                self.assertTrue(urlsplit(config["endpoint"]).hostname.endswith(".invalid"))
                self.assertEqual(config["audience"], "receiver-approved-google-oauth-client-id")
                self.assertEqual(set(config), {"version", "endpoint", "audience", "allowed_orgs", "app_slug"})

    def test_shared_configuration_rejects_credentials_and_local_identities(self):
        config = json.loads((SAMPLES / "meshly.sample.json").read_text())
        schema = json.loads((SAMPLES / "shared-config.schema.json").read_text())
        self.assertIs(schema["additionalProperties"], False)
        self.assertEqual(set(schema["properties"]), set(config))
        for forbidden in ("private_key", "webhook_secret", "client_secret", "token", "installation_token",
                          "github_account", "google_account", "developer_authorization_records"):
            with self.subTest(field=forbidden), self.assertRaises(bootstrap.SetupError):
                bootstrap.encode_bundle({**config, forbidden: "synthetic-rejected-value"})

    def test_receiver_operator_contract_is_not_a_developer_bundle(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "pack", "--config-file",
                                 str(SAMPLES / "receiver-inputs.sample.json")],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")
        self.assertIn(b"unsupported fields", result.stderr)

    def test_receiver_contract_keeps_secrets_and_principals_unprovisioned(self):
        contract = json.loads((SAMPLES / "receiver-inputs.sample.json").read_text())
        self.assertIs(contract["sample_only"], True)
        for reference in contract["proposed_secret_manager_references"].values():
            self.assertEqual(reference["version"], "TO_BE_PROVISIONED")
            self.assertEqual(set(reference), {"project_id", "secret_id", "version"})
            self.assertTrue(reference["secret_id"].startswith("github-events-"))
        for record in contract["developer_authorization_records"]:
            self.assertEqual(record["status"], "DISABLED_SAMPLE_ONLY_NOT_AN_ACCESS_GRANT")
            self.assertTrue(record["google_identity"]["subject"].startswith("ILLUSTRATIVE_"))
            self.assertTrue(record["github_identity"]["user_id"].startswith("ILLUSTRATIVE_"))
            self.assertTrue(record["github_identity"]["login"].startswith("ILLUSTRATIVE_"))
            self.assertTrue(record["authorization_evidence"]["source"].startswith("TO_BE_PROVISIONED_"))
        self.assertFalse(contract["production_bootstrap_object"]["object_name"].startswith("everyone/github-events/samples/"))

    def test_schema_declares_strict_shared_structure_and_matching_patterns(self):
        schema = json.loads((SAMPLES / "shared-config.schema.json").read_text())
        self.assertEqual(schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
        self.assertEqual(schema["type"], "object")
        self.assertIs(schema["additionalProperties"], False)
        self.assertEqual(set(schema["required"]), {"version", "endpoint", "audience", "allowed_orgs"})
        fields = schema["properties"]
        self.assertEqual(fields["version"], {"type": "integer", "const": 1})
        self.assertEqual(fields["endpoint"]["format"], "uri")
        self.assertIs(fields["allowed_orgs"]["uniqueItems"], True)
        self.assertEqual((fields["allowed_orgs"]["minItems"], fields["allowed_orgs"]["maxItems"]), (1, 100))
        for name in ("meshly", "selfhost"):
            with self.subTest(sample=name):
                config = json.loads((SAMPLES / f"{name}.sample.json").read_text())
                for field in ("endpoint", "audience", "app_slug"):
                    self.assertRegex(config[field], fields[field]["pattern"])
                for org in config["allowed_orgs"]:
                    self.assertRegex(org, fields["allowed_orgs"]["items"]["pattern"])
                bootstrap.encode_bundle(config)

    def test_schema_endpoint_patterns_and_real_packer_reject_unsafe_values(self):
        schema = json.loads((SAMPLES / "shared-config.schema.json").read_text())
        config = json.loads((SAMPLES / "meshly.sample.json").read_text())
        bad_endpoints = (
            "http://example.invalid", "https://user:password@example.invalid", "https://example.invalid?query=value",
            "https://example.invalid#fragment", "https://example.invalid:0", "https://example.invalid/path with space",
            "https://example.invalid\\path", "https://example.invalid/path\u0085", "https://example.invalid/path\u0090")
        endpoint_schema = schema["properties"]["endpoint"]
        for endpoint in bad_endpoints:
            with self.subTest(endpoint=endpoint):
                accepted = (re.search(endpoint_schema["pattern"], endpoint) is not None
                            and re.search(endpoint_schema["not"]["pattern"], endpoint) is None)
                self.assertFalse(accepted)
        bad = [{**config, "endpoint": endpoint} for endpoint in bad_endpoints]
        bad += [{**config, "allowed_orgs": ["bad--org"]}, {**config, "version": True}]
        for value in bad:
            with self.subTest(value=value), self.assertRaises(bootstrap.SetupError):
                bootstrap.encode_bundle(value)


if __name__ == "__main__":
    unittest.main()
