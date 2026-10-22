#!/usr/bin/env python3
"""Real desktop CSV + URL acceptance. Build `cargo build -p notoir` first.
Uses a disposable profile, real rendered-widget input, files and HTTP server.
The Save As destination is supplied through ScriptedDialogs; no OS automation.
Artifacts: screenshots, UI snapshots, assertions and the desktop process log.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/notoir')
    parser.add_argument('--artifacts', default='target/live-documents')
    parser.add_argument('--https-url', help='Also download a public HTTPS text document')
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    (artifacts / 'results.json').unlink(missing_ok=True)
    source = 'Name;City;Value\r\nAlice;Porto;old\r\nBob;Lisbon;old\r\nAlina;Lisbon;other\r\n'

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == '/redirect':
                self.send_response(302)
                self.send_header('Location', '/remote.csv')
                self.end_headers()
                return
            if self.path == '/slow':
                time.sleep(1.5)
            if self.path == '/missing':
                self.send_error(404)
                return
            body = b'Name,Value\r\nRemote,original\r\n'
            self.send_response(200)
            self.send_header('Content-Type', 'text/csv')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def log_message(self, *_):
            pass

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f'http://127.0.0.1:{server.server_port}'
    results = []
    with tempfile.TemporaryDirectory(prefix='notoir-live-documents-') as root:
        root = Path(root)
        file = root / 'table.csv'
        file.write_bytes(source.encode())
        env = dict(os.environ, NOTOIR_HOME=str(root / 'profile'), NOTOIR_LANG='en',
                   NOTOIR_TEST_COMMANDS='1', NOTOIR_LOG='info')
        log = (artifacts / 'desktop.log').open('w')
        process = subprocess.Popen([binary, str(file)], env=env,
                                   stdout=log, stderr=log)
        sequence = 0

        def send(data, delay=.12):
            subprocess.run([binary, '--command', 'test.ui', '--args', json.dumps(data)],
                           env=env, check=True, timeout=15, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL)
            time.sleep(delay)

        def snapshot():
            nonlocal sequence
            sequence += 1
            output = artifacts / f'state-{sequence:03}.json'
            output.unlink(missing_ok=True)
            send({'op': 'snapshot', 'output': str(output)})
            until = time.monotonic() + 10
            while time.monotonic() < until:
                try:
                    return json.loads(output.read_text())
                except (FileNotFoundError, json.JSONDecodeError):
                    if process.poll() is not None:
                        raise AssertionError('Desktop process exited; see desktop.log')
                    time.sleep(.05)
            raise AssertionError('Desktop snapshot timed out')

        def check(condition, name):
            assert condition, name
            results.append(name)
            print('PASS:', name, flush=True)

        def click(target, double=False):
            state = snapshot()
            assert any(w['label'] == target for w in state['widgets']), (target, state['widgets'])
            if double:
                time.sleep(.4)
            send({'op': 'doubleClick' if double else 'click', 'target': target})

        def fill(target, text):
            click(target)
            send({'op': 'key', 'key': 'A', 'command': True})
            send({'op': 'text', 'text': text})

        def capture(name):
            path = artifacts / f'{name}.png'
            path.unlink(missing_ok=True)
            send({'op': 'capture', 'path': str(path)})
            until = time.monotonic() + 8
            while not path.exists() and time.monotonic() < until:
                time.sleep(.1)
            assert path.exists(), f'screenshot missing: {path}'

        def url_dialog(path):
            click('menus:top.file')
            click('file.openFromUrl')
            fill('url.address', path if path.startswith('https://') else base + path)

        try:
            time.sleep(2)
            state = snapshot()
            check(any(d['name'] == 'table.csv' for d in state['documents']), 'Real desktop opened CSV file')
            click('csv.showPreview')
            state = snapshot()
            check(state['csv']['separator'] == ';' and state['csv']['header'], 'CSV detected semicolon and default header')
            capture('csv-preview')
            click('csv.showSplit')
            check(snapshot()['mode'] == 'CsvSplit', 'Split mode renders source and table')
            click('csv.showPreview')
            click('csv.header')
            check(not snapshot()['csv']['header'], 'Header checkbox displays first row as data')
            click('csv.header')
            click('csv.separator')
            click('csv:comma')
            check(snapshot()['csv']['separator'] == ',', 'Separator override reparses CSV')
            click('csv.separator')
            click('csv:semicolon')
            click('csv.columns')
            click('csv.column.1')
            check(1 in snapshot()['csv']['hidden'], 'Column can be hidden')
            send({'op': 'key', 'key': 'Escape'})
            click('csv.columns')
            click('csv.showAll')
            send({'op': 'key', 'key': 'Escape'})
            check(not snapshot()['csv']['hidden'], 'Show All restores columns')
            click('csv.filter.1')
            click('csv.hideColumn')
            check(1 in snapshot()['csv']['hidden'], 'Hide column action works inside the column popover')
            click('csv.columns')
            click('csv.showAll')
            send({'op': 'key', 'key': 'Escape'})
            fill('csv.search', 'ALI')
            state = snapshot()
            check(len(state['csv']['matches']) == 2 and len(state['csv']['rows']) == 3,
                  'Search highlights multiple values without hiding rows')
            click('csv.next')
            click('csv.previous')
            click('csv.filter.1')
            fill('csv.filterInput', 'lis')
            send({'op': 'key', 'key': 'Escape'})
            check(snapshot()['csv']['rows'] == [2, 3], 'Column filter narrows rows case-insensitively')
            click('csv.filter.0')
            fill('csv.filterInput', 'ali')
            send({'op': 'key', 'key': 'Escape'})
            check(snapshot()['csv']['rows'] == [3], 'Column filters combine with AND')
            capture('csv-filtered')
            click('csv.filter.0')
            click('csv.clearFilter')
            send({'op': 'key', 'key': 'Escape'})
            click('csv.filter.1')
            click('csv.clearFilter')
            send({'op': 'key', 'key': 'Escape'})
            fill('csv.search', '')
            # Empty Text events cannot erase input; clear through Backspace after Select All.
            send({'op': 'key', 'key': 'Backspace'})
            click('csv.cell.1.2', double=True)
            check(snapshot()['csv']['editing'], 'Double-click opens a real inline cell editor')
            fill('csv.editCell', 'new;"value')
            send({'op': 'key', 'key': 'Enter'})
            state = snapshot()
            expected = source.replace('Alice;Porto;old', 'Alice;Porto;"new;""value"')
            check(state['documents'][0]['text'] == expected.replace('\r\n', '\n'), 'Cell edit escapes separator and quotes')
            send({'op': 'key', 'key': 'Z', 'command': True})
            check(snapshot()['documents'][0]['text'] == source.replace('\r\n', '\n'), 'Desktop Undo reverses one cell edit')
            send({'op': 'key', 'key': 'Z', 'command': True, 'shift': True})
            check(snapshot()['documents'][0]['text'] == expected.replace('\r\n', '\n'), 'Desktop Redo restores cell edit')
            click('menus:top.file')
            click('file.save')
            check(file.read_bytes() == expected.encode(), 'Save writes exact CSV with original CRLF')
            capture('csv-edited')
            url_dialog('/redirect')
            capture('url-dialog')
            click('url.open')
            state = snapshot()
            until = time.monotonic() + 10
            while state['url_busy'] and time.monotonic() < until:
                state = snapshot()
            remote = next(d for d in state['documents'] if d['name'] == 'remote.csv')
            check(remote['source_url'] == base + '/remote.csv' and remote['dirty'] and remote['path'] is None,
                  'URL redirect opens editable unsaved CSV with final filename')
            click('csv.showPreview')
            click('csv.cell.1.1', double=True)
            fill('csv.editCell', 'download edited')
            send({'op': 'key', 'key': 'Enter'})
            destination = root / 'download.csv'
            send({'op': 'savePath', 'path': str(destination)})
            click('menus:top.file')
            click('file.save')
            check(destination.read_bytes() == b'Name,Value\r\nRemote,download edited\r\n',
                  'Downloaded CSV edited in Preview and saved locally')
            url_dialog('/missing')
            click('url.open')
            state = snapshot()
            check(state['url_error'] is not None and len(state['documents']) == 2, 'Failed download displays error without creating a tab')
            capture('url-error')
            click('url.cancel')
            url_dialog('/slow')
            click('url.open')
            click('url.cancel')
            time.sleep(1.8)
            state = snapshot()
            check(not state['url_open'] and len(state['documents']) == 2, 'Cancelled download ignores late response')
            if args.https_url:
                url_dialog(args.https_url)
                click('url.open')
                state=snapshot()
                until=time.monotonic()+35
                while state['url_busy'] and time.monotonic()<until:
                    state=snapshot()
                check(not state['url_open'] and not state['url_error'] and any(d['source_url'] and d['source_url'].startswith('https://') for d in state['documents']),
                      'Public HTTPS document downloaded with TLS verification')
                capture('url-https')
            # Close/reopen saved CSV through the same instance, checking disk loading and reset presentation.
            subprocess.run([binary, str(file)], env=env, check=True, timeout=15,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(.2)
            click('menus:top.file')
            click('file.close')
            subprocess.run([binary, str(file)], env=env, check=True, timeout=15,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(.2)
            state=snapshot()
            check(state['mode']=='Text' and next(d for d in state['documents'] if d['name']=='table.csv')['text']==expected.replace('\r\n','\n'),
                  'Reopening CSV loads saved bytes and resets the view mode')
            click('csv.showPreview')
            check(snapshot()['csv']['header'] and not snapshot()['csv']['hidden'], 'CSV presentation choices reset on reopen')
            (artifacts / 'results.json').write_text(json.dumps({'passed': results, 'save_picker': 'scripted local destination',
                                                              'input': 'pointer and keyboard events in running desktop app'}, indent=2))
        finally:
            process.terminate()
            try:
                process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            log.close()
            server.shutdown()
    print(f'Live desktop tests passed: {len(results)}. Artifacts: {artifacts}', flush=True)


if __name__ == '__main__':
    main()
