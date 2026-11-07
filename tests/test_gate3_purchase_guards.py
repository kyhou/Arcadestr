import hashlib
from pathlib import Path
import runpy
import tempfile
import subprocess
import time
import unittest
from unittest.mock import patch

M = runpy.run_path(str(Path(__file__).resolve().parents[1]/'scripts/regtest-gate3-purchase.py'))


class PurchaseGuards(unittest.TestCase):
    def test_retired_budget_wizard_exits_before_any_setup(self):
        script=Path(__file__).resolve().parents[1]/'scripts/regtest-gate3-budget-wizard.sh'
        with tempfile.TemporaryDirectory() as directory:
            result=subprocess.run(['bash',str(script)],cwd=directory,capture_output=True,text=True)
            self.assertEqual(result.returncode,1)
            self.assertIn('retired wallet-specific',result.stderr)
            self.assertEqual(list(Path(directory).iterdir()),[])

    def test_retired_live_driver_cannot_resume_without_pause_marker(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError,'Retired wallet-specific'):
                M['permissions'](Path(directory))

    def setUp(self):
        self.nodes={name:{'identity_pubkey':name} for name in ('alice','bob','carol','dave')}
        self.decoded={'destination':'bob','num_msat':'100000','payment_hash':'new',
                      'timestamp':int(time.time()),'expiry':3600}

    def test_invoice_limits_and_destinations(self):
        M['validate_invoice']('lnbcrt1fixture',self.decoded,self.nodes,{})
        for change in [{'destination':'alice'},{'destination':'dave'},{'num_msat':'100001'},{'expiry':30}]:
            with self.assertRaises(RuntimeError):
                M['validate_invoice']('lnbcrt1fixture',{**self.decoded,**change},self.nodes,{})
        with self.assertRaises(RuntimeError):
            M['validate_invoice']('lnbc1fixture',self.decoded,self.nodes,{})
        for seen in [{'new':'carol'},{'previous':'bob'}]:
            with self.assertRaises(RuntimeError):
                M['validate_invoice']('lnbcrt1fixture',self.decoded,self.nodes,seen)

    def test_incomplete_history_is_never_payment_authority(self):
        function=M['history']
        for response in [{'payments':[]},{'payments':[],'total_num_payments':'1'},
                         {'payments':[{'status':'UNKNOWN','payment_hash':'x'}],'total_num_payments':'1'}]:
            with patch.dict(function.__globals__,{'R':{'lncli':lambda *args:response}}):
                with self.assertRaises(RuntimeError):
                    function()

    def test_failed_or_invalid_wallet_proof_cannot_advance(self):
        function=M['paid']
        invoice={'payment_hash':hashlib.sha256(bytes(32)).hexdigest()}
        for row in [{'payment_hash':invoice['payment_hash'],'status':'FAILED'},
                    {'payment_hash':invoice['payment_hash'],'status':'SUCCEEDED','value_msat':'100000','payment_preimage':'01'*32}]:
            with patch.dict(function.__globals__,{'history':lambda:[row]}):
                with self.assertRaises(RuntimeError):
                    function(invoice)
        with patch.dict(function.__globals__,{'history':lambda:[]}):
            self.assertIsNone(function(invoice))

    def test_used_admission_never_reaches_gui_pay(self):
        function=M['admit']
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            (root/'phase3-new-purchase').mkdir()
            (root/'phase3-new-purchase/dispatch-0.admitted.json').write_text('{}')
            def forbidden(*args):
                self.fail('Used admission reached invoice or GUI dispatch')
            with patch.dict(function.__globals__,{'preflight':lambda *args:({}, {}, {}),
                                                 'current_invoice':forbidden}):
                with self.assertRaisesRegex(RuntimeError,'admission already used'):
                    function(root,0)

    def test_user_pause_blocks_before_reading_approval_or_wallet(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            (root/'phase3-new-purchase').mkdir()
            (root/'phase3-new-purchase/paused-by-user.json').write_text('{}')
            with self.assertRaisesRegex(RuntimeError,'paused by user'):
                M['permissions'](root)
