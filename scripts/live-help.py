#!/usr/bin/env python3
"""Live Help acceptance on macOS. Needs Python cryptography and a built desktop binary.
Runs the actual app with pointer events, a signed HTTP fixture, Finder Open With
Apple Events, an Automator service, and a harmless disk image installer handoff.
Uses a disposable profile, app bundle, service directory and disk image.
"""
import argparse
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import plistlib
import shutil
import shlex
import subprocess
import tempfile
import threading
import time
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/notoir')
    parser.add_argument('--artifacts', default='target/live-help')
    parser.add_argument('--native-service', action='store_true', help='Temporarily register the service in the real per-user Services folder; refuses any existing workflow')
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    (artifacts / 'results.json').unlink(missing_ok=True)
    signing = Ed25519PrivateKey.from_private_bytes(bytes([7]) * 32)
    keyid = bytes([7]) * 8
    public = base64.b64encode(b'Ed' + keyid + signing.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)).decode()
    results = []
    mode = ['new']
    with tempfile.TemporaryDirectory(prefix='notoir-live-help-') as temporary:
        root = Path(temporary)
        dmgroot = root / 'image'
        dmgroot.mkdir()
        (dmgroot / 'Live test only.txt').write_text('Harmless update handoff fixture. No application is replaced.')
        dmg = root / 'Notoir-live-test.dmg'
        subprocess.run(['hdiutil', 'create', '-srcfolder', str(dmgroot), '-volname', 'Notoir Live Update Test', '-format', 'UDZO', str(dmg)], check=True, stdout=subprocess.DEVNULL)
        installer = dmg.read_bytes()

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                if mode[0]=='unavailable':
                    self.send_error(503)
                    return
                sha = hashlib.sha256(installer if mode[0] != 'tampered' else b'wrong').hexdigest()
                version = '99.0.0' if mode[0] != 'current' else '0.0.1'
                manifest = json.dumps({'version': version, 'artifacts': {'macos': {'url': base + '/installer.dmg', 'sha256': sha}}}).encode()
                signature = signing.sign(manifest)
                trusted = b'timestamp:0\tfile:update.json'
                sigtext = ('untrusted comment: live fixture\n' + base64.b64encode(b'Ed' + keyid + signature).decode() + '\ntrusted comment: ' + trusted.decode() + '\n' + base64.b64encode(signing.sign(signature + trusted)).decode() + '\n').encode()
                if mode[0] == 'bad':
                    sigtext = b'invalid signature'
                body = installer if self.path == '/installer.dmg' else sigtext if self.path.endswith('.minisig') else manifest
                self.send_response(200)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def log_message(self, *_):
                pass

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        base = f'http://127.0.0.1:{server.server_port}'
        threading.Thread(target=server.serve_forever, daemon=True).start()
        bundle = root / 'Notoir Live.app'
        contents = bundle / 'Contents'
        (contents / 'MacOS').mkdir(parents=True)
        appbin = contents / 'MacOS/notoir'
        shutil.copy2(binary, appbin)
        (contents / 'Info.plist').write_bytes(plistlib.dumps({
            'CFBundleIdentifier': 'dev.notoir.live-help', 'CFBundleName': 'Notoir Live',
            'CFBundleExecutable': 'notoir', 'CFBundlePackageType': 'APPL',
            'CFBundleDocumentTypes': [{'CFBundleTypeExtensions': ['txt'], 'CFBundleTypeRole': 'Editor', 'CFBundleTypeName': 'Text'}]
        }))
        env = dict(os.environ, NOTOIR_HOME=str(root / 'profile'), NOTOIR_LANG='en', NOTOIR_TEST_COMMANDS='1')
        log = (artifacts / 'desktop.log').open('w')
        process = subprocess.Popen([str(appbin)], env=env, stdout=log, stderr=log)
        wrapper=root / 'service-launcher'
        wrapper.write_text('#!/bin/sh\nexport NOTOIR_HOME='+shlex.quote(env['NOTOIR_HOME'])+'\nexport NOTOIR_TEST_COMMANDS=1\nexec '+shlex.quote(str(appbin))+' "$@"\n')
        wrapper.chmod(0o755)
        service_home=Path.home() if args.native_service else root/'service-home'
        workflow=service_home/'Library/Services/Open with Notoir.workflow'
        if args.native_service and workflow.exists():
            process.terminate()
            process.wait(timeout=10)
            raise RuntimeError('Existing Open with Notoir workflow: refusing to replace it in a live test')
        sequence = 0

        def send(data, delay=.16):
            subprocess.run([str(appbin), '--command', 'test.ui', '--args', json.dumps(data)], env=env, check=True, timeout=15, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(delay)

        def snapshot():
            nonlocal sequence
            sequence += 1
            path = artifacts / f'state-{sequence:03}.json'
            path.unlink(missing_ok=True)
            send({'op': 'snapshot', 'output': str(path)})
            end = time.monotonic() + 10
            while time.monotonic() < end:
                try:
                    return json.loads(path.read_text())
                except (FileNotFoundError, json.JSONDecodeError):
                    time.sleep(.08)
            raise AssertionError('snapshot timed out; see desktop.log')

        def check(condition, name):
            assert condition, name
            results.append(name)
            print('PASS:', name, flush=True)

        def wait(predicate):
            end = time.monotonic() + 15
            while time.monotonic() < end:
                state = snapshot()
                if predicate(state):
                    return state
            raise AssertionError('condition timed out')

        def click(target):
            state = snapshot()
            assert any(w['label'] == target for w in state['widgets']), (target, state['widgets'])
            send({'op': 'click', 'target': target})

        def capture(name):
            path = artifacts / (name + '.png')
            path.unlink(missing_ok=True)
            send({'op': 'capture', 'path': str(path)})
            end = time.monotonic() + 8
            while not path.exists() and time.monotonic() < end:
                time.sleep(.1)
            assert path.exists()

        def check_updates():
            click('menus:top.help')
            click('app.checkForUpdates')
            return wait(lambda s: any(w['label'] in ('app.checkForUpdates','app.quitAndInstall') for w in s['widgets']))

        def command(id, args=None):
            subprocess.run([str(appbin), '--command', id, '--args', json.dumps(args)],env=env,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            time.sleep(.2)

        try:
            time.sleep(2)
            send({'op': 'configureHelp', 'integrationHome': str(service_home), 'integrationTarget': str(wrapper), 'updateUrl': base + '/update.json', 'publicKey': public})
            wait(lambda s: not s['integration_busy'])
            click('menus:top.help')
            click('app.addContextMenu')
            wait(lambda s: s['integration_installed'] is True and not s['integration_busy'])
            check(True, 'Help installs per-user integration and reports status')
            capture('file-manager-integration')
            if args.native_service:
                dump=subprocess.check_output(['/System/Library/CoreServices/pbs','-dump'],text=True)
                check('Open with Notoir' in dump,'Finder Services cache includes the registered Open with Notoir action')
            for path in (workflow / 'Contents').iterdir():
                plistlib.loads(path.read_bytes())
            check(True, 'Finder service property lists are valid')
            click('integration.close')
            servicefile = root / "a file's service & text.txt"
            servicefile.write_text('service opened this')
            result = subprocess.run(['automator', '-i', str(servicefile), str(workflow)], env=env, capture_output=True, text=True, timeout=30)
            (artifacts / 'automator.log').write_text(result.stdout + result.stderr)
            check(result.returncode == 0, 'Native Automator action executes without osascript')
            wait(lambda s: any(d['name'] == servicefile.name for d in s['documents']))
            check(True, 'Finder service opens a path with spaces, apostrophe and ampersand')
            finderfile = root / 'finder open.txt'
            finderfile.write_text('Finder Apple Event')
            secondfile=root / 'second file.txt'
            secondfile.write_text('Second Apple Event file')
            subprocess.run(['open', '-a', str(bundle), str(finderfile),str(secondfile)], check=True, env=env)
            wait(lambda s: any(d['name'] == finderfile.name for d in s['documents']))
            wait(lambda s:any(d['name']==secondfile.name for d in s['documents']))
            check(True, 'Finder Open With opens multiple files in the running application')
            folder=root/'workspace'
            folder.mkdir()
            result=subprocess.run(['automator','-i',str(folder),str(workflow)],env=env,capture_output=True,text=True,timeout=30)
            check(result.returncode==0,'Native Service accepts a folder')
            wait(lambda s:s.get('workspace')==str(folder.resolve()))
            check(True,'Folder opens as a workspace')
            click('menus:top.help')
            click('app.removeContextMenu')
            wait(lambda s: s['integration_installed'] is False and not s['integration_busy'])
            check(not (workflow / 'Contents/Info.plist').exists(), 'Help removes owned integration')
            click('integration.close')
            mode[0] = 'current'
            check_updates()
            check(snapshot()['updates_open'], 'Manual check shows current-version result')
            capture('updates-current')
            click('app.closeUpdates')
            mode[0]='unavailable'
            check_updates()
            check(snapshot()['update_error'] is not None,'Network failure is reported and can be retried')
            click('app.closeUpdates')
            mode[0] = 'bad'
            check_updates()
            check(snapshot()['update_error'] is not None, 'Bad signature is rejected in desktop update dialog')
            capture('updates-error')
            click('app.closeUpdates')
            mode[0] = 'tampered'
            check_updates()
            click('app.downloadUpdate')
            wait(lambda s: s['update_error'] is not None)
            check(snapshot()['update_ready'] is None, 'Checksum mismatch cannot enable installation')
            click('app.closeUpdates')
            mode[0] = 'new'
            check_updates()
            capture('updates-available')
            click('app.downloadUpdate')
            state = wait(lambda s: s['update_ready'] is not None)
            check(Path(state['update_ready']).read_bytes() == installer, 'Download Update verifies the signed fixture bytes')
            capture('updates-ready')
            click('app.closeUpdates')
            command('file.newUntitled')
            command('editor.type',{'text':'unsaved live text'})
            check(any(d['text']=='unsaved live text' and d['dirty'] for d in snapshot()['documents']),'Live fixture contains an unsaved document')
            check_updates()
            click('app.quitAndInstall')
            wait(lambda s:s['prompt_count']>0)
            click('prompt.Cancel')
            state=snapshot()
            check(not state['install_on_quit'] and state['update_ready'] is not None and any(d['text']=='unsaved live text' and d['dirty'] for d in state['documents']),'Cancelling Quit and Install preserves unsaved text and verified update')
            check_updates()
            click('app.quitAndInstall')
            wait(lambda s:s['prompt_count']>0)
            capture('updates-unsaved-prompt')
            click('prompt.Secondary')
            process.wait(timeout=20)
            check(process.returncode == 0, 'Quit and Install shuts down normally')
            mounted = False
            for _ in range(40):
                info = plistlib.loads(subprocess.check_output(['hdiutil', 'info', '-plist']))
                mounted = any('Notoir Live Update Test' in str(image) for image in info.get('images', []))
                if mounted:
                    break
                time.sleep(.25)
            check(mounted, 'Verified disk image is handed to macOS after shutdown')
            # Cold Open With: LaunchServices must deliver the first document too.
            cold = root / 'cold finder.txt'
            cold.write_text('cold start')
            cold_launcher=subprocess.Popen(['open', '-W', '-n', '-a', str(bundle), '--env', 'NOTOIR_HOME=' + env['NOTOIR_HOME'], '--env', 'NOTOIR_TEST_COMMANDS=1', str(cold)])
            time.sleep(2)
            wait(lambda s: any(d['name'] == cold.name for d in s['documents']))
            check(True, 'Finder Open With delivers documents on cold launch')
            subprocess.run([str(appbin), '--command', 'app.quit'], env=env, check=True)
            cold_launcher.wait(timeout=20)
            (artifacts / 'results.json').write_text(json.dumps({'passed': len(results), 'checks': results, 'input': 'pointer events in running desktop app; native Apple Events and Automator', 'installer': 'harmless real DMG handed to macOS'}, indent=2) + '\n')
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
            subprocess.run(['hdiutil', 'detach', '/Volumes/Notoir Live Update Test'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if args.native_service:
                for filename in ['Info.plist','document.wflow']:
                    path=workflow/'Contents'/filename
                    if path.exists() and 'dev.notoir.context.v1' in path.read_text():
                        path.unlink()
                for path in [workflow/'Contents',workflow]:
                    try:
                        path.rmdir()
                    except (FileNotFoundError,OSError):
                        pass
                subprocess.run(['/System/Library/CoreServices/pbs','-update'],check=True)
            server.shutdown()
            log.close()
    print(f'{len(results)} live checks passed; artifacts: {artifacts}')


if __name__ == '__main__':
    main()
