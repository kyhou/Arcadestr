import runpy
from pathlib import Path
import unittest

VALIDATE = runpy.run_path(str(Path(__file__).resolve().parents[1]/'scripts/regtest-gate3-nip46.py'))['validate']
COMPLETE_HISTORY = runpy.run_path(str(Path(__file__).resolve().parents[1]/'scripts/regtest-gate3-nip46.py'))['complete_history']


class HistoryGuards(unittest.TestCase):
    def test_incomplete_lookup_never_authorizes_dispatch(self):
        with self.assertRaises(RuntimeError):
            COMPLETE_HISTORY({'lncli': lambda *args: {'payments': [], 'total_num_payments': '1'}})

    def test_missing_count_never_authorizes_dispatch(self):
        with self.assertRaises(RuntimeError):
            COMPLETE_HISTORY({'lncli': lambda *args: {'payments': []}})

    def test_unknown_payment_status_never_authorizes_dispatch(self):
        with self.assertRaises(RuntimeError):
            COMPLETE_HISTORY({'lncli': lambda *args: {'payments': [{'payment_hash': 'one', 'status': 'UNKNOWN'}], 'total_num_payments': '1'}})

    def test_complete_empty_lookup_requires_counting_flag(self):
        def lncli(*args):
            self.assertIn('--count_total_payments', args)
            self.assertIn('--include_incomplete', args)
            return {'payments': [], 'total_num_payments': '0'}
        self.assertEqual(COMPLETE_HISTORY({'lncli': lncli}), [])


class DestinationGuards(unittest.TestCase):
    def setUp(self):
        self.nodes = {name: {'identity_pubkey': name} for name in ('alice', 'bob', 'carol', 'dave')}
        self.invoice = {'destination': 'alice', 'num_msat': '100000', 'payment_hash': 'one'}

    def check_rejected(self, invoice=None, bolt11='lnbcrt1fixture', seen=None):
        with self.assertRaises(RuntimeError):
            VALIDATE(invoice or self.invoice, bolt11, self.nodes, 'bob', seen or {})

    def test_existing_regtest_destination(self):
        VALIDATE(self.invoice, 'lnbcrt1fixture', self.nodes, 'bob', {})

    def test_self_payment_rejected(self):
        self.check_rejected({**self.invoice, 'destination': 'bob'})

    def test_unknown_node_rejected(self):
        self.check_rejected({**self.invoice, 'destination': 'external'})

    def test_wrong_recipient_rejected(self):
        self.check_rejected({**self.invoice, 'destination': 'dave'})

    def test_wrong_amount_rejected(self):
        self.check_rejected({**self.invoice, 'num_msat': '100001'})

    def test_mainnet_rejected(self):
        self.check_rejected(bolt11='lnbc1fixture')

    def test_duplicate_hash_rejected(self):
        self.check_rejected(seen={'one': 'carol'})

    def test_duplicate_recipient_rejected(self):
        self.check_rejected(seen={'other': 'alice'})
