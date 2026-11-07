#!/usr/bin/env python3
"""Gate 3 integration helpers. No payment command is implemented here."""
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys

REPO = Path(__file__).resolve().parent.parent
GUI = runpy.run_path(str(REPO/'scripts/regtest-gate3-denial.py'))
REGTEST = runpy.run_path(str(REPO/'scripts/regtest'))


def private_environment(root):
    matches = []
    for path in Path('/proc').iterdir():
        if not path.name.isdigit():
            continue
        try:
            if 'arcadestr' not in (path/'comm').read_text():
                continue
            env = dict(item.split(b'=',1) for item in (path/'environ').read_bytes().split(b'\0') if b'=' in item)
            if env.get(b'XDG_DATA_HOME') == str(root/'desktop-data').encode():
                matches.append(env)
        except OSError:
            continue
    if len(matches) != 1:
        raise RuntimeError('Expected exactly one owned private desktop')
    environment = os.environ.copy()
    for key in ['HOME','DBUS_SESSION_BUS_ADDRESS','XDG_DATA_HOME','XDG_CONFIG_HOME','XDG_CACHE_HOME',
                'ARCADESTR_REGTEST_ALLOW_LOOPBACK_HTTP','ARCADESTR_REGTEST_NWC_RELAY_PROXY']:
        if key.encode() in matches[0]:
            environment[key] = matches[0][key.encode()].decode()
        else:
            environment.pop(key, None)
    return environment


def invoke(root, command, args=None, timeout=30):
    sid = json.loads((root/'gui-session.json').read_text())['session_id']
    script = ('const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('
        +json.dumps(command)+','+json.dumps(args or {})+')'
        '.then(value=>done({ok:true,value})).catch(error=>done({ok:false,failure:String(error)}));')
    return GUI['request'](f'/session/{sid}/execute/async', {'script':script,'args':[]}, timeout=timeout)


def audit_nwc_binding(root, after_restart=False):
    environment = private_environment(root)
    info = REGTEST['lncli']('alice','getinfo')
    if info['chains'] != [{'chain':'bitcoin','network':'regtest'}]:
        raise RuntimeError('Existing NWC wallet node is not regtest')
    environment.update(ARCADESTR_REGTEST_INTEGRATION_ROOT=str(root),
        ARCADESTR_REGTEST_BUYER_PUBKEY=GUI['ncli'](root,'id','gate3-denial')['pub_hex'],
        ARCADESTR_REGTEST_EXPECTED_LND_PUBKEY=info['identity_pubkey'])
    name = 'phase2-nwc-binding-after-restart.json' if after_restart else 'phase2-nwc-binding.json'
    environment['ARCADESTR_REGTEST_BINDING_AUDIT_NAME'] = name
    result = subprocess.run([str(REPO/'target/debug/examples/regtest_gate3_nwc_binding')], env=environment,
        capture_output=True, text=True, timeout=30)
    GUI['save'](root, name+'.command.json', {'returncode':result.returncode,'stdout':result.stdout,'stderr':result.stderr})
    if result.returncode:
        raise RuntimeError('Read-only NWC binding audit blocked; inspect private evidence')
    if after_restart:
        before = json.loads((root/'phase2-nwc-binding.json').read_text())
        after = json.loads((root/name).read_text())
        for key in ['buyer','wallet_pubkey','application_pubkey','payment_identity','lnd_pubkey']:
            if before[key] != after[key]:
                raise RuntimeError('Persisted NWC identity binding changed after restart')


def wallet_verify(root, success):
    GUI['js'](root,"[...document.querySelectorAll('button')].find(b=>b.innerText.trim()==='VERIFY CONNECTION').click();return true;")
    text = 'Configured · verified this session' if success else 'Configured · verification failed'
    GUI['wait_gui'](root,'document.body.innerText.includes('+json.dumps(text)+')',timeout=25)


def readonly_faults(root):
    proxy = root/'phase2-nwc-proxy'
    before = json.loads((proxy/'network-audit.json').read_text())['requests_forwarded']
    GUI['save'](proxy,'network-mode.json',{'mode':'offline','phase':'phase2-readonly-outage'})
    # The proxy also rejects new handshakes immediately in offline mode.
    wallet_verify(root,False)
    GUI['snapshot'](root,'phase2-nwc-unavailable-gui')
    after = json.loads((proxy/'network-audit.json').read_text())['requests_forwarded']
    if before != after:
        raise RuntimeError('Unexpected request forwarding during controlled NWC outage')
    GUI['save'](proxy,'network-mode.json',{'mode':'online','phase':'phase2-readonly-reconnected'})
    wallet_verify(root,True)
    GUI['snapshot'](root,'phase2-nwc-reconnected-gui')
    GUI['ncli'](root,'bunker','stop')
    wallet_verify(root,True)
    if not GUI['js'](root,'return !!document.querySelector("[data-testid=regtest-nip44-probe]")'):
        GUI['js'](root,"[...document.querySelectorAll('button')].find(b=>b.innerText.trim()==='Debug').click();return true;")
    GUI['js'](root,"document.querySelector('[data-testid=regtest-nip44-probe]').click();return true;")
    GUI['wait_gui'](root,'!document.querySelector("[data-testid=regtest-nip44-probe]").disabled && document.querySelector("[data-testid=regtest-nip44-result]").innerText.includes("timed out")',timeout=80)
    GUI['snapshot'](root,'phase2-signer-unavailable-wallet-binding-retained')
    GUI['ncli'](root,'bunker','--identity','gate3-denial','--relay','ws://127.0.0.1:7778')
    GUI['js'](root,"document.querySelector('[data-testid=regtest-nip44-probe]').click();return true;")
    GUI['wait_gui'](root,'!document.querySelector("[data-testid=regtest-nip44-probe]").disabled && document.querySelector("[data-testid=regtest-nip44-result]").innerText.includes("NIP-44 decryption verified.")',timeout=45)
    wallet_verify(root,True)
    GUI['snapshot'](root,'phase2-signer-and-wallet-restored')
    report = invoke(root,'reconcile_pending_settlements')
    if not report['ok']:
        raise RuntimeError('Read-only empty-queue reconciliation failed')
    GUI['save'](root,'phase2-empty-queue-reconcile.json',report)
    GUI['save'](root,'phase2-readonly-fault-verification.json',{'nwc_outage_retains_binding':True,
        'outage_forwarded_requests':0,'signer_unavailable_blocks_decryption':True,
        'wallet_read_only_verification_independent_of_signer':True,'same_signer_restarted':True,
        'payments':0,'durable_paid_intent_verified':False})


if __name__ == '__main__':
    os.umask(0o077)
    if len(sys.argv)!=3 or sys.argv[1] not in ('binding-after-restart','readonly-faults'):
        raise SystemExit('usage: regtest-gate3-integration.py {binding-after-restart|readonly-faults} PRIVATE_ROOT')
    root = Path(sys.argv[2]).resolve(strict=True)
    if sys.argv[1]=='binding-after-restart':
        audit_nwc_binding(root,True)
    else:
        readonly_faults(root)
    print('PASS read-only integration checks; no payment operation implemented or dispatched')
