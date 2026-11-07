import hashlib
import json
from pathlib import Path
import runpy
import sqlite3
import tempfile
import unittest

FUNCTIONS = runpy.run_path(str(Path(__file__).resolve().parents[1]/'scripts/regtest-gate3-nwc-validate.py'))
URI = 'nostr+walletconnect://'+'ab'*32+'?relay=ws%3A%2F%2F127.0.0.1%3A7777&secret='+'0'*63+'1'


class ReadOnlyCapture(unittest.TestCase):
    def test_retired_budget_gate_does_not_access_wallet_or_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError,'Retired'):
                FUNCTIONS['validate'](Path(directory),'not-a-connection',approved_budget=True)

    def test_user_pause_blocks_budget_setup_before_connection_access(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            (root/'phase3-new-purchase').mkdir()
            (root/'phase3-new-purchase/paused-by-user.json').write_text('{}')
            with self.assertRaisesRegex(ValueError,'User paused'):
                FUNCTIONS['validate'](root,'not-a-connection',approved_budget=True)

    def test_public_relay_is_rejected(self):
        with self.assertRaises(ValueError):
            FUNCTIONS['parse_identity'](URI.replace('127.0.0.1', 'remote.example'))

    def test_duplicate_secret_is_rejected(self):
        with self.assertRaises(ValueError):
            FUNCTIONS['parse_identity'](URI+'&secret='+'0'*63+'2')

    def test_fragment_is_rejected(self):
        with self.assertRaises(ValueError):
            FUNCTIONS['parse_identity'](URI+'#unexpected')

    def test_existing_application_is_rejected_and_readonly_new_app_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root/'hub.db'
            c = sqlite3.connect(path)
            c.execute('CREATE TABLE apps(id INTEGER,name TEXT,app_pubkey TEXT,wallet_pubkey TEXT,isolated INTEGER)')
            c.execute('CREATE TABLE app_permissions(id INTEGER,app_id INTEGER,scope TEXT)')
            c.execute('CREATE TABLE transactions(id INTEGER,value INTEGER)')
            _, app, _ = FUNCTIONS['parse_identity'](URI)
            c.execute('INSERT INTO apps VALUES(3,?,?,?,0)', ('existing',app,'ab'*32))
            baseline = {}
            for table in ['apps','app_permissions','transactions']:
                cols = [r[1] for r in c.execute('PRAGMA table_info('+table+')')]
                rows = c.execute('SELECT * FROM '+table).fetchall()
                baseline[table] = {'columns':cols,'rows':{str(row[0]):hashlib.sha256(json.dumps(row,sort_keys=True).encode()).hexdigest() for row in rows}}
            (root/'original-hub-binding-and-history-baseline.json').write_text(json.dumps(baseline))
            c.commit()
            with self.assertRaisesRegex(ValueError, 'newly created dedicated'):
                FUNCTIONS['validate'](root,URI,path)
            # Use a second protected fixture key, not an existing application's credential.
            second = URI[:-1]+'2'
            _, public, _ = FUNCTIONS['parse_identity'](second)
            c.execute('INSERT INTO apps VALUES(4,?,?,?,0)', ('dedicated',public,'ab'*32))
            for index, scope in enumerate(['get_info','get_balance','lookup_invoice','list_transactions']):
                c.execute('INSERT INTO app_permissions VALUES(?,?,?)', (index+1,4,scope))
            c.commit()
            result = FUNCTIONS['validate'](root,second,path)
            self.assertEqual(result['app_id'],4)
            self.assertFalse(result['payment_permission'])
            c.execute("INSERT INTO app_permissions VALUES(99,4,'pay_invoice')")
            c.commit()
            with self.assertRaisesRegex(ValueError, 'four requested read-only'):
                FUNCTIONS['validate'](root,second,path)
            c.execute("UPDATE apps SET name='changed' WHERE id=3")
            c.commit()
            with self.assertRaisesRegex(ValueError, 'existing Hub binding'):
                FUNCTIONS['validate'](root,second,path)
            c.execute("UPDATE apps SET name='existing' WHERE id=3")
            c.execute('DELETE FROM app_permissions WHERE id=99')
            c.commit()
            with self.assertRaisesRegex(ValueError,'Retired'):
                FUNCTIONS['validate'](root,second,path,approved_budget=True)
            c.close()
