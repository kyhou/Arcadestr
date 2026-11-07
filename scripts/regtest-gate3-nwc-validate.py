#!/usr/bin/env python3
"""Historical read-only fixture audit, not an Arcadestr wallet adapter or payment gate."""
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import sys
from urllib.parse import parse_qs, urlsplit


def parse_identity(uri):
    from cryptography.hazmat.primitives.asymmetric import ec
    parsed = urlsplit(uri)
    if (parsed.scheme != 'nostr+walletconnect' or parsed.path or parsed.fragment
            or not re.fullmatch('[0-9a-fA-F]{64}', parsed.netloc)):
        raise ValueError('Expected a Nostr Wallet Connect URI')
    fields = parse_qs(parsed.query, keep_blank_values=True)
    secrets = fields.get('secret', [])
    if len(secrets) != 1 or not re.fullmatch('[0-9a-fA-F]{64}', secrets[0]):
        raise ValueError('Invalid application secret encoding')
    relays = fields.get('relay', [])
    if not relays:
        raise ValueError('Existing local relay required')
    for relay in relays:
        target = urlsplit(relay)
        if (target.scheme != 'ws' or target.hostname not in ('127.0.0.1', 'localhost', '::1')
                or target.port not in (7777, 7778) or target.username or target.password
                or target.query or target.fragment or target.path not in ('', '/')):
            raise ValueError('Only existing loopback regtest relays are allowed')
    private = ec.derive_private_key(int(secrets[0], 16), ec.SECP256K1())
    public = format(private.public_key().public_numbers().x, '064x')
    return parsed.netloc.lower(), public, relays


def validate(root, uri, hub_path=None, approved_budget=False):
    if approved_budget and (root/'phase3-new-purchase/paused-by-user.json').exists():
        raise ValueError('User paused payment setup; explicit resumption required')
    if approved_budget:
        raise ValueError('Retired wallet-specific spending validation; use Arcadestr purchase approval')
    wallet, application, relays = parse_identity(uri)
    original = json.loads((root/'original-hub-binding-and-history-baseline.json').read_text())
    path = hub_path or Path.home()/'.local/share/arcadestr-albyhub-regtest/nwc.db'
    c = sqlite3.connect(f'file:{path}?mode=ro', uri=True)
    for table, baseline in original.items():
        cols = baseline['columns']
        for row in c.execute('SELECT '+','.join(cols)+' FROM '+table):
            identity = str(row[cols.index('id')])
            if identity in baseline['rows']:
                digest = hashlib.sha256(json.dumps(row, default=lambda v:bytes(v).hex(), sort_keys=True).encode()).hexdigest()
                if digest != baseline['rows'][identity]:
                    raise ValueError('An existing Hub binding, permission or transaction changed; stop')
        present = {str(row[0]) for row in c.execute('SELECT id FROM '+table)}
        if not set(baseline['rows']).issubset(present):
            raise ValueError('An existing Hub record is missing; stop')
    rows = c.execute('SELECT id,name,wallet_pubkey,isolated FROM apps WHERE app_pubkey=?', (application,)).fetchall()
    if len(rows) != 1 or str(rows[0][0]) in original['apps']['rows']:
        raise ValueError('URI must belong to a newly created dedicated application, not Alice’s existing app')
    app_id, name, expected_wallet, isolated = rows[0]
    if expected_wallet != wallet:
        raise ValueError('Wallet/application binding mismatch')
    if isolated:
        raise ValueError('Create a regular dedicated app, not an unfunded isolated sub-wallet')
    scopes = {row[0] for row in c.execute('SELECT scope FROM app_permissions WHERE app_id=?', (app_id,))}
    expected_scopes = {'get_info','get_balance','lookup_invoice','list_transactions'}
    if scopes != expected_scopes:
        raise ValueError('Use only the four requested read-only permissions; no payment, invoice creation or superuser scope')
    return {'app_id':app_id,'name':name,'wallet_pubkey':wallet,'application_pubkey':application,
            'relays':relays,'scopes':sorted(scopes),'existing_hub_bindings_and_history_unchanged':True,
            'payment_permission':approved_budget,'payment_budget_approved':approved_budget}


if __name__ == '__main__':
    os.umask(0o077)
    try:
        root = Path(sys.argv[1]).resolve(strict=True)
        if root.stat().st_uid != os.getuid() or root.stat().st_mode & 0o077:
            raise ValueError('Owner-only run directory required')
        paid = len(sys.argv)==3 and sys.argv[2]=='--budget'
        if paid:
            raise ValueError('Retired spending setup; no credential access or wallet changes allowed')
        uri = (root/'nwc-setup.env').read_text().strip().split('=',1)[1] if paid else sys.stdin.read().strip()
        result = validate(root, uri, approved_budget=paid)
        path = root/('dedicated-nwc-budget-validation.json' if paid else 'dedicated-nwc-readonly-validation.json')
        if path.exists():
            if json.loads(path.read_text()) != result:
                raise ValueError('Different application already captured; preserve it')
        else:
            with path.open('x') as output:
                json.dump(result, output, indent=2)
        path.chmod(0o600)
        print('PASS dedicated application binding; existing application/history preserved; '+
              ('exact approved 250-sat non-renewing cap' if paid else 'no payment permission'))
    except (ValueError, sqlite3.Error, OSError, IndexError, ImportError):
        print('BLOCKED: NWC validation failed; URI not stored or disclosed. Keep the app read-only and ask for assistance.', file=sys.stderr)
        sys.exit(1)
