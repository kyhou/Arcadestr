"""Non-paying regressions for the manual reboot/fault harness."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


def load(name, file):
    path = Path(__file__).resolve().parents[1] / 'scripts' / file
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


remaining = load('gate3_remaining', 'regtest-gate3-remaining.py')
network = load('gate3_network', 'regtest-gate3-network.py')


class Gate3RemainingTests(unittest.TestCase):
    def test_missing_private_signer_storage_is_not_regenerated(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            with self.assertRaisesRegex(RuntimeError,'refusing regeneration'):
                remaining.private_credentials(root)
            self.assertFalse((root/'desktop-data').exists())

    def test_sqlite_encrypted_blob_fingerprints_support_bytes_and_text(self):
        self.assertEqual(remaining.encrypted_blob_digest(b'ciphertext'), remaining.encrypted_blob_digest('ciphertext'))
        self.assertEqual(remaining.encrypted_blob_digest(None), remaining.encrypted_blob_digest(b''))

    def test_application_restart_cannot_be_reported_as_machine_reboot(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            remaining.save(root / 'reboot-checkpoint.json', {'boot_id': remaining.boot_id()})
            with patch.object(remaining, 'launch') as launch, patch.object(remaining.subprocess, 'run') as run:
                with self.assertRaisesRegex(RuntimeError, 'Same boot'):
                    remaining.after_reboot(root)
            launch.assert_not_called()
            run.assert_not_called()
            result = json.loads((root / 'reboot-verification.json').read_text())
            self.assertFalse(result['machine_reboot_verified'])
            self.assertEqual(result['payment_dispatches'], 0)

    def test_fault_filter_only_drops_response_events_not_payment_requests(self):
        request = ['EVENT', {'kind': 23194, 'content': 'opaque encrypted payload'}]
        response = ['EVENT', 'subscription', {'kind': 23195, 'content': 'opaque encrypted payload'}]
        self.assertFalse(network.response_event(json.dumps(request)))
        self.assertTrue(network.response_event(json.dumps(response)))
        for value in ['{}', 'null', 'bad JSON', '["OK", "id", true]', '["EVENT", {}, {}]']:
            self.assertFalse(network.response_event(value))

    def test_copy_boundary_cannot_follow_symlink_to_original_database(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as outside:
            root = Path(directory)
            target = root / 'desktop-data' / 'arcadestr'
            target.mkdir(parents=True)
            original = Path(outside) / 'original.db'
            original.write_bytes(b'not to be opened')
            (target / 'arcadestr.db').symlink_to(original)
            with self.assertRaises(RuntimeError):
                remaining.database(root, readonly=False)
            self.assertEqual(original.read_bytes(), b'not to be opened')


if __name__ == '__main__':
    unittest.main()
