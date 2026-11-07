import json
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch

MODULE = runpy.run_path(str(Path(__file__).resolve().parents[1]/'scripts/regtest-gate3-denial.py'))


class DenialHarnessGuards(unittest.TestCase):
    def test_ambiguous_pending_request_never_gets_a_decision(self):
        function = MODULE['pending']
        calls = []
        def ncli(*args):
            calls.append(args)
            return [{'id': '1', 'method': 'sign_event'}, {'id': '2', 'method': 'sign_event'}]
        with tempfile.TemporaryDirectory() as directory, patch.dict(function.__globals__, {'ncli': ncli}):
            with self.assertRaisesRegex(RuntimeError, 'Ambiguous'):
                function(Path(directory), 'sign_event')
        self.assertEqual(len(calls), 1)

    def test_never_approves_another_clients_handshake(self):
        function = MODULE['pending']
        calls = []
        def ncli(*args):
            calls.append(args)
            return [{'id': '1', 'method': 'connect', 'app': 'foreign'},
                    {'id': '2', 'method': 'sign_event', 'app': 'ours'}]
        with tempfile.TemporaryDirectory() as directory, patch.dict(function.__globals__, {'ncli': ncli}):
            root = Path(directory)
            (root/'sign-initial-request.json').write_text(json.dumps({'app': 'ours'}))
            self.assertEqual(function(root, 'sign_event')['id'], '2')
        self.assertEqual(len(calls), 1)

    def test_revoke_only_requested_method_and_kind(self):
        function = MODULE['remove_grant']
        grants = [{'method': 'sign_event', 'kind': 27235}, {'method': 'nip44_decrypt'}]
        calls = []
        def ncli(root, *args):
            calls.append(args)
            if 'revoke-grant' in args:
                self.assertEqual(args[-4:], ('--method', 'sign_event', '--kind', '27235'))
                grants.pop(0)
                return {}
            return list(grants)
        with patch.dict(function.__globals__, {'ncli': ncli}):
            self.assertEqual(function(Path('/unused'), 'ours', 'sign_event'), [{'method': 'nip44_decrypt'}])
        self.assertEqual(len(calls), 3)

    def test_refuses_to_continue_when_grant_remains(self):
        function = MODULE['remove_grant']
        with patch.dict(function.__globals__, {'ncli': lambda *args: [{'method': 'nip44_decrypt'}]}):
            with self.assertRaisesRegex(RuntimeError, 'Auto-approval grant remains'):
                function(Path('/unused'), 'ours', 'nip44_decrypt')

    def test_protected_state_mismatch_stops_before_signer_access(self):
        function = MODULE['verify']
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            protected = root/'original-journal'
            protected.write_text('preserve this')
            (root/'protected-baseline.json').write_text(json.dumps({str(protected): 'wrong-digest'}))
            def forbidden(*args):
                self.fail('Signer accessed after protected-state mismatch')
            with patch.dict(function.__globals__, {'ncli': forbidden}):
                with self.assertRaisesRegex(RuntimeError, 'Protected existing state changed'):
                    function(root)
            self.assertEqual(protected.read_text(), 'preserve this')
