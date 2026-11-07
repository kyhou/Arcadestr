"""Supervisor safety regressions. No Docker, wallets, payments or live services."""
import importlib.machinery
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

loader = importlib.machinery.SourceFileLoader('regtest', str(Path(__file__).resolve().parents[1] / 'scripts/regtest'))
spec = importlib.util.spec_from_loader(loader.name, loader)
regtest = importlib.util.module_from_spec(spec)
loader.exec_module(regtest)


class SafetyTests(unittest.TestCase):
    def test_gate2_incomplete_reservation_blocks_new_run_and_verification(self):
        with tempfile.TemporaryDirectory() as tmp:
            state=Path(tmp)
            original=json.dumps({'report':str(state/'unfinished.json'),'state':'reserved_before_payment'})
            reservation=state/'gate2-reservation.json';reservation.write_text(original)
            with patch.object(regtest,'STATE',state),patch.object(regtest,'verify_bitcoin'), \
                 patch.object(regtest,'verify_nodes',return_value={}),patch.object(regtest.subprocess,'run') as command:
                for verification in [False,True]:
                    budget=regtest.PaymentBudget()
                    with self.assertRaises(regtest.Blocked):
                        regtest.gate2_scenarios(budget,verification)
                    self.assertEqual(budget.total,0)
                command.assert_not_called()
            self.assertEqual(reservation.read_text(),original)

    def test_gate2_pre_payment_continuation_refuses_existing_order_or_intent_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            state=Path(tmp); root=state/'original'; stage=root/'success';stage.mkdir(parents=True)
            (stage/'purchase.json').write_text(json.dumps({'state':'payment_submitting'}))
            reservation=state/'gate2-reservation.json'
            original=json.dumps({'report':str(root/'report.json')});reservation.write_text(original)
            with patch.object(regtest,'STATE',state),patch.object(regtest,'verify_bitcoin'), \
                 patch.object(regtest,'verify_nodes',return_value={}),patch.object(regtest.subprocess,'run') as command:
                with self.assertRaises(regtest.Blocked):regtest.gate2_scenarios(regtest.PaymentBudget(),continuation=True)
                command.assert_not_called()
            self.assertEqual(reservation.read_text(),original)

    def test_dave_profile_does_not_inherit_carol_secrets_or_fee_overlay(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'dave.env'
            values={'ADP_OPERATOR_NSEC':'synthetic-dave','ADP_MASTER_KEY':'09'*32,
                    'ADP_DB_PATH':str(Path(tmp)/'dave.db'),'ADP_FILES_DIR':str(Path(tmp)/'files'),
                    'ADP_TERMS_EVENT_ID':'02'*32,'ADP_SERVER_URL':'http://127.0.0.1:19100',
                    'ADP_BIND_ADDR':'127.0.0.1:19100','ADP_RELAYS':'ws://127.0.0.1:7777',
                    'ADP_PAYMENT_DESTINATION':'dave@127.0.0.1:18080'}
            path.write_text(''.join(f'{k}={v}\n' for k,v in values.items()));path.chmod(0o600)
            carol={'ADP_OPERATOR_NSEC':'synthetic-carol','ADP_MASTER_KEY':'07'*32,'ADP_DB_PATH':str(Path(tmp)/'carol.db')}
            with patch.object(regtest,'settings',return_value={'dave_adp_env':str(path)}), \
                 patch.object(regtest,'adp_env',return_value=carol), \
                 patch.dict(regtest.os.environ,{'ADP_OPERATOR_NSEC':'synthetic-carol','ADP_FEE_POLICY':'must-not-inherit'}):
                env=regtest.dave_env(Path(tmp))
                self.assertEqual(env['ADP_OPERATOR_NSEC'],'synthetic-dave')
                self.assertNotIn('ADP_FEE_POLICY',env)
                values['ADP_MASTER_KEY']=carol['ADP_MASTER_KEY']
                path.write_text(''.join(f'{k}={v}\n' for k,v in values.items()))
                with self.assertRaises(regtest.Blocked):regtest.dave_env(Path(tmp))

    def test_unresolved_gate1_reservation_never_launches_or_clears(self):
        with tempfile.TemporaryDirectory() as tmp:
            state=Path(tmp)
            reservation=state/'gate1-reservation.json'
            original=json.dumps({'report':str(state/'missing-report.json'),'state':'reserved_before_payment'})
            reservation.write_text(original)
            budget=regtest.PaymentBudget()
            with patch.object(regtest,'STATE',state),patch.object(regtest,'verify_bitcoin'), \
                 patch.object(regtest,'verify_nodes',return_value={}),patch.object(regtest,'probe'), \
                 patch.object(regtest,'verify_nwc'),patch.object(regtest.subprocess,'run') as command:
                with self.assertRaises(regtest.Blocked):
                    regtest.gate1_scenarios(budget)
                command.assert_not_called()
            self.assertEqual(budget.total,0)
            self.assertEqual(reservation.read_text(),original)

    def test_refund_verification_cannot_resume_an_incomplete_suite(self):
        with tempfile.TemporaryDirectory() as tmp:
            state=Path(tmp)
            report=state/'incomplete.json'
            report.write_text(json.dumps({'status':'pending','cases':[]}))
            (state/'refund-reservation.json').write_text(json.dumps({'report':str(report)}))
            with patch.object(regtest,'STATE',state),patch.object(regtest,'lncli') as command:
                with self.assertRaises(regtest.Blocked):
                    regtest.verify_refunds()
                command.assert_not_called()

    def test_incomplete_refund_suite_cannot_launch_new_payments(self):
        with tempfile.TemporaryDirectory() as tmp:
            state=Path(tmp)
            (state/'refund-reservation.json').write_text(json.dumps({'report':str(state/'missing-report.json')}))
            budget=regtest.PaymentBudget()
            with patch.object(regtest,'STATE',state),patch.object(regtest,'verify_bitcoin'), \
                 patch.object(regtest,'verify_nodes',return_value={}),patch.object(regtest,'probe'), \
                 patch.object(regtest,'verify_nwc'),patch.object(regtest.subprocess,'run') as command:
                with self.assertRaises(regtest.Blocked):
                    regtest.refund_scenarios(budget)
                command.assert_not_called()
            self.assertEqual(budget.total,0)

    def test_verification_only_never_resumes_a_partially_paid_crash_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            state=Path(tmp)
            report=state/'pending.json'
            report.write_text(json.dumps({'state':'payment_submitting','payments':[{'state':'reserved'}]}))
            (state/'durability-reservation.json').write_text(json.dumps({'report':str(report)}))
            budget=regtest.PaymentBudget()
            with patch.object(regtest,'STATE',state),patch.object(regtest,'verify_bitcoin'), \
                 patch.object(regtest,'verify_nodes',return_value={}),patch.object(regtest,'probe'), \
                 patch.object(regtest,'verify_nwc'),patch.object(regtest,'fixture_env',return_value={}), \
                 patch.object(regtest,'settings',return_value={'adp_repo':tmp}), \
                 patch.object(regtest,'adp_env',return_value={'ADP_OPERATOR_NSEC':'synthetic','ADP_DB_PATH':'data.db'}), \
                 patch.object(regtest.subprocess,'run',return_value=SimpleNamespace(returncode=0,stdout=b'{"state":"complete"}',stderr=b'')) as command:
                with self.assertRaises(regtest.Blocked):
                    regtest.split_purchase(budget,verification_only=True,crash_recovery=True)
                self.assertEqual(command.call_count,1,'only compilation is permitted; no resume/payment subprocess')
            self.assertEqual(budget.total,0)

    def test_purchase_limit_is_across_all_legs(self):
        budget = regtest.PaymentBudget()
        budget.reserve('purchase', 6_000_000)
        with self.assertRaises(regtest.Blocked):
            budget.reserve('purchase', 4_000_001)
        self.assertEqual(budget.total, 6_000_000)

    def test_run_limit_includes_failed_or_ambiguous_attempts(self):
        budget = regtest.PaymentBudget()
        for index in range(5):
            budget.reserve(str(index), 10_000_000)
        with self.assertRaises(regtest.Blocked):
            budget.reserve('sixth', 1)

    def test_invalid_amounts(self):
        for amount in [0, -1, 1.5, '1000', True]:
            with self.subTest(amount=amount), self.assertRaises(regtest.Blocked):
                regtest.PaymentBudget().reserve('q', amount)

    def test_only_loopback_urls(self):
        for url in ['http://127.0.0.1:9099', 'http://[::1]:9099', 'ws://localhost:7777']:
            regtest.local_url(url)
        for url in ['http://example.com', 'http://localhost.evil.test', 'http://192.168.0.1',
                    'http://127.0.0.1@example.com', 'http://user:secret@localhost',
                    'file:///etc/passwd', 'https://localhost/#secret']:
            with self.subTest(url=url), self.assertRaises(regtest.Blocked):
                regtest.local_url(url)

    def test_non_regtest_nodes_fail_closed(self):
        for network in ['bitcoin', 'testnet', 'signet']:
            with patch.object(regtest, 'lncli', return_value={'chains': [{'chain': 'bitcoin', 'network': network}]}):
                with self.assertRaises(regtest.Blocked):
                    regtest.verify_nodes()

    def test_unfunded_channels_fail_closed(self):
        def cli(node, method):
            if method == 'getinfo':
                return {'chains': [{'chain':'bitcoin','network':'regtest'}], 'synced_to_chain': True,
                        'identity_pubkey': node}
            return {'channels': []}
        with patch.object(regtest, 'lncli', side_effect=cli), self.assertRaises(regtest.Blocked):
            regtest.verify_nodes()

    def test_rejects_external_peer_liquidity(self):
        def cli(node, method):
            if method == 'getinfo':
                return {'chains': [{'chain':'bitcoin','network':'regtest'}], 'synced_to_chain': True,
                        'identity_pubkey': node}
            return {'channels':[{'active':True,'remote_pubkey':'outside','local_balance':'999999','remote_balance':'999999'}]}
        with patch.object(regtest, 'lncli', side_effect=cli), self.assertRaises(regtest.Blocked):
            regtest.verify_nodes()

    def test_invoice_network_destination_amount_and_expiry(self):
        valid = {'destination':'local', 'num_msat':'1000', 'timestamp':'100','expiry':'60'}
        regtest.validate_invoice('lnbcrt10n1signed', valid, 'local', 1000, now=159)
        for invoice, decoded in [('lnbc10n1signed', valid), ('lntb10n1signed', valid),
                                 ('lnbcrt10n1signed', dict(valid, destination='outside')),
                                 ('lnbcrt10n1signed', dict(valid, num_msat='1001')),
                                 ('lnbcrt10n1signed', dict(valid, expiry='59'))]:
            with self.subTest(invoice=invoice, decoded=decoded), self.assertRaises(regtest.Blocked):
                regtest.validate_invoice(invoice, decoded, 'local', 1000, now=159)

    def test_reboot_and_reused_pids_are_never_owned(self):
        entry = {'pid': 123, 'identity':['old-boot','1']}
        for current in [None, ['new-boot','1'], ['old-boot','2']]:
            with patch.object(regtest, 'process_identity', return_value=current):
                self.assertFalse(regtest.owned(entry))

    def test_shutdown_never_signals_unowned_processes(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(regtest, 'STATE', Path(tmp)), \
                 patch.object(regtest, 'read_state', return_value={'adp':{'pid':123,'identity':['old','1']}}), \
                 patch.object(regtest, 'process_identity', return_value=['new','1']), \
                 patch.object(regtest.os, 'kill') as kill:
                self.assertFalse(regtest.down(None))
                kill.assert_not_called()

    def test_healthy_unowned_listener_is_not_claimed_or_spawned(self):
        with patch.object(regtest, 'read_state', return_value={}), \
             patch.object(regtest, 'listening', return_value=True), \
             patch.object(regtest, 'probe'), \
             patch.object(regtest.subprocess, 'Popen') as spawn:
            regtest.start('adp', ['not-executed'], '.')
            spawn.assert_not_called()

    def test_port_collision_is_not_killed_or_replaced(self):
        with patch.object(regtest, 'read_state', return_value={}), \
             patch.object(regtest, 'listening', return_value=True), \
             patch.object(regtest, 'probe', side_effect=regtest.Blocked('wrong service')), \
             patch.object(regtest.subprocess, 'Popen') as spawn:
            with self.assertRaises(regtest.Blocked):
                regtest.start('adp', ['not-executed'], '.')
            spawn.assert_not_called()

    def test_empty_fixture_env_does_not_create_identities(self):
        with tempfile.TemporaryDirectory() as tmp:
            config = Path(tmp)
            env = config / 'lnurl-fixture.env'
            env.write_text('')
            env.chmod(0o600)
            infos = {n: {'identity_pubkey':n} for n in regtest.NODES}
            with patch.object(regtest, 'CONFIG', config), patch.dict(regtest.os.environ, {}, clear=True):
                with self.assertRaisesRegex(regtest.Blocked, 'missing existing alice'):
                    regtest.fixture_env(infos)
            self.assertEqual(env.read_text(), '')

    def test_fixture_env_is_data_not_executable_shell(self):
        with tempfile.TemporaryDirectory() as tmp:
            config = Path(tmp)
            env = config / 'lnurl-fixture.env'
            env.write_text('POLAR_ALICE_NOSTR_PUBKEY=$(touch /not/executed)')
            env.chmod(0o600)
            with patch.object(regtest, 'CONFIG', config), self.assertRaises(regtest.Blocked):
                regtest.fixture_env({})

    def test_fixture_env_expands_existing_root_without_executing_shell(self):
        with tempfile.TemporaryDirectory() as tmp:
            config = Path(tmp)
            env = config / 'lnurl-fixture.env'
            lines = ['export POLAR_LND_ROOT="$HOME/existing-polar"']
            lines += [f'export POLAR_{n.upper()}_NOSTR_PUBKEY=' + 'a'*64 for n in regtest.NODES]
            original = '\n'.join(lines)
            env.write_text(original)
            env.chmod(0o600)
            infos = {n:{'identity_pubkey':n} for n in regtest.NODES}
            with patch.object(regtest, 'CONFIG', config), patch.dict(regtest.os.environ, {'HOME':'/existing-home'}, clear=True):
                result = regtest.fixture_env(infos)
            self.assertEqual(result['POLAR_ALICE_TLS_CERT'], '/existing-home/existing-polar/alice/tls.cert')
            self.assertEqual(env.read_text(), original)

    def test_adp_requires_existing_database_and_local_relays(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = root / '.env'
            values = {'ADP_OPERATOR_NSEC':'not-a-real-secret','ADP_MASTER_KEY':'not-a-real-key',
                      'ADP_DB_PATH':'missing.db','ADP_FILES_DIR':'files',
                      'ADP_SERVER_URL':'http://localhost:9099', 'ADP_RELAYS':'ws://localhost:7777,ws://localhost:7778'}
            env.write_text('\n'.join(f'{k}={v}' for k,v in values.items()))
            with patch.object(regtest, 'settings', return_value={}), self.assertRaisesRegex(regtest.Blocked, 'database missing'):
                regtest.adp_env(root)
            (root/'missing.db').touch()
            values['ADP_RELAYS'] = 'wss://external.example'
            env.write_text('\n'.join(f'{k}={v}' for k,v in values.items()))
            with patch.object(regtest, 'settings', return_value={}), self.assertRaises(regtest.Blocked):
                regtest.adp_env(root)

    def test_atomic_state_is_owner_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/'state.json'
            regtest.save(path, {'ok':True})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(json.loads(path.read_text()), {'ok':True})

    def test_payment_rejects_unknown_destination_before_reserving_or_paying(self):
        def cli(node, method, *args):
            if method == 'addinvoice':
                return {'payment_request':'lnbcrt1fake'}
            if method == 'decodepayreq':
                return {'destination':'outside','num_msat':'100000','timestamp':str(int(regtest.time.time())), 'expiry':'120'}
            self.fail('must never submit a payment')
        infos = {n:{'identity_pubkey':n} for n in regtest.NODES}
        budget = regtest.PaymentBudget()
        with patch.object(regtest, 'verify_bitcoin'), patch.object(regtest, 'verify_nodes', return_value=infos), \
             patch.object(regtest, 'lncli', side_effect=cli), self.assertRaises(regtest.Blocked):
            regtest.lightning_smoke(budget, {'payments':[]}, 'alice', 'bob', 'p')
        self.assertEqual(budget.total, 0)

    def test_payment_reservation_is_durable_before_submission(self):
        preimage = '01' * 32
        payment_hash = regtest.hashlib.sha256(bytes.fromhex(preimage)).hexdigest()
        budget = regtest.PaymentBudget()
        ledger = {'payments':[]}
        with tempfile.TemporaryDirectory() as tmp:
            state = Path(tmp)
            def cli(node, method, *args):
                if method == 'addinvoice':
                    return {'payment_request':'lnbcrt1fake'}
                if method == 'decodepayreq':
                    return {'destination':'bob', 'num_msat':'100000','timestamp':str(int(regtest.time.time())),
                            'expiry':'120', 'payment_hash':payment_hash}
                if method == 'payinvoice':
                    saved = json.loads((state/'payment-run.json').read_text())
                    self.assertEqual(saved['reserved_msat'], 110000)
                    self.assertEqual(saved['payments'][0]['state'], 'reserved')
                    self.assertEqual(budget.total, 110000)
                    self.assertIn('--fee_limit=10', args)
                    return {'status':'SUCCEEDED','payment_preimage':preimage}
                if method == 'lookupinvoice':
                    return {'settled':True,'amt_paid_msat':'100000'}
                self.fail('unexpected wallet method')
            infos = {n:{'identity_pubkey':n} for n in regtest.NODES}
            with patch.object(regtest, 'STATE', state), patch.object(regtest, 'verify_bitcoin'), \
                 patch.object(regtest, 'verify_nodes', return_value=infos), patch.object(regtest, 'lncli', side_effect=cli):
                regtest.lightning_smoke(budget, ledger, 'alice', 'bob', 'p')
            self.assertEqual(ledger['payments'][0]['state'], 'settled')
            self.assertNotIn(preimage, (state/'payment-run.json').read_text())


if __name__ == '__main__':
    unittest.main()
