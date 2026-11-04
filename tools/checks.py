#!/usr/bin/env python3
"""Owner's check dashboard: lists docs/handover.md items and runs their commands, one at a time.

Serves tools/checks.html on 127.0.0.1 only. Commands come from the handover table, never from the client.
Run from your own terminal: python3 tools/checks.py --open
"""
import argparse
import json
import os
import re
import shlex
import signal
import subprocess
import threading
import time
import webbrowser
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

ROOT = Path(__file__).resolve().parent.parent
PAGE = Path(__file__).resolve().with_name('checks.html')
STATE = ROOT / 'target' / 'checks_state.json'
HANDOVER = ROOT / 'docs' / 'handover.md'
LOCK = threading.Lock()
proc = None          # Popen of the current or last run
item = None          # item index of that run
cmd_index = None
cmd = None           # command string of that run
started = time.time()
run_exit = None      # return code once the run ends, None while running
lines = []           # output, last 2000 lines
last = {}           # command string -> {exit, finished}


def label_of(c):
    if ' test ' in c:
        return 'test'
    return 'shot' if '--shot' in c else 'live'


def is_test_fn(name):
    return any(re.search(rf'fn {name}\b', t.read_text()) for t in (ROOT / 'tests').glob('*.rs'))


def variant(base, span):
    """base with the span's flags: the example reads the first --pose/--shot, so a repeated flag replaces its value; --set appends (the later one wins)."""
    toks, add = shlex.split(base), shlex.split(span)
    if '--' not in toks:
        toks.append('--')  # flags after this go to the example, not to cargo
    out = []
    while add:
        flag = add.pop(0)
        val = add.pop(0) if add and not add[0].startswith('--') else None
        if flag != '--set' and val is not None and flag in toks:
            toks[toks.index(flag) + 1] = val
        else:
            out += [flag] + ([val] if val is not None else [])
    return shlex.join(toks + out)


def parse(path):
    items = []
    change = ''
    for line in Path(path).read_text().splitlines():
        if not line.startswith('| 20'):
            continue
        cells = [c.strip() for c in line.strip().strip('|').split('|')]
        date, chg, run, look, status = (cells + [''] * 5)[:5]
        if chg == 'Same':
            chg = change
        change = chg
        cmds = []
        base_label = None
        for span in re.findall(r'`([^`]+)`', run):
            if span.startswith('cargo '):
                base_label = label_of(span)
                cmds.append({'label': base_label, 'cmd': span})
            elif span.startswith('--') and cmds and base_label != 'test':
                cmds.append({'label': base_label + ' (variant)', 'cmd': variant(cmds[-1]['cmd'], span)})
            elif re.fullmatch(r'[a-z0-9_]+', span) and cmds and base_label == 'test' and is_test_fn(span):
                # "then `cost_rain`": the same test command with another test name as its filter
                cmds.append({'label': 'test', 'cmd': cmds[-1]['cmd'].rsplit(' ', 1)[0] + ' ' + span + ' --exact'})
        items.append({'date': date, 'change': chg, 'look': look, 'status': status, 'cmds': cmds})
    return items


def pngs_for(c, since):
    if not c or '--out' not in shlex.split(c):
        return []
    toks = shlex.split(c)
    k = toks.index('--out')
    if k + 1 >= len(toks) or not (ROOT / toks[k + 1]).is_dir():
        return []
    found = [p for p in (ROOT / toks[k + 1]).rglob('*.png') if p.stat().st_mtime >= since - 1]
    found.sort(key=lambda p: p.stat().st_mtime, reverse=True)
    return [p.relative_to(ROOT).as_posix() for p in found[:40]]


def save_state():
    STATE.parent.mkdir(parents=True, exist_ok=True)
    STATE.write_text(json.dumps(last, indent=1))


def reader(p, c):
    global run_exit
    for line in p.stdout:
        with LOCK:
            lines.append(line.rstrip('\n'))
            del lines[:-2000]
    rc = p.wait()
    with LOCK:
        run_exit = rc
        last[c] = {'exit': rc, 'finished': datetime.now().isoformat(timespec='seconds')}
        save_state()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def _send(self, code, body, ctype='application/json'):
        if not isinstance(body, bytes):
            body = json.dumps(body).encode()
        self.send_response(code)
        self.send_header('Content-Type', ctype)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        url = urlparse(self.path)
        if url.path == '/':
            self._send(200, PAGE.read_bytes(), 'text/html; charset=utf-8')
        elif url.path == '/items':
            items = parse(HANDOVER)
            with LOCK:
                for it in items:
                    for c in it['cmds']:
                        c['last'] = last.get(c['cmd'])
            self._send(200, items)
        elif url.path == '/status':
            with LOCK:
                running = proc is not None and run_exit is None
                body = {'running': running, 'item': item, 'cmd_index': cmd_index, 'cmd': cmd,
                        'started': started, 'exit': run_exit,
                        'output': '\n'.join(lines[-400:]), 'pngs': pngs_for(cmd, started)}
            self._send(200, body)
        elif url.path == '/file':
            p = parse_qs(url.query).get('p', [''])[0]
            target = (ROOT / p).resolve()
            if p.endswith('.png') and target.is_relative_to(ROOT) and target.is_file():
                self._send(200, target.read_bytes(), 'image/png')
            else:
                self._send(404, {'error': 'not found'})
        else:
            self._send(404, {'error': 'not found'})

    def do_POST(self):
        global proc, item, cmd_index, cmd, started, run_exit, lines
        if self.path == '/stop':
            with LOCK:
                if proc is not None and run_exit is None:
                    try:
                        os.killpg(proc.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
            self._send(200, {'ok': True})
            return
        if self.path != '/run':
            self._send(404, {'error': 'not found'})
            return
        if self.headers.get('Content-Type', '').split(';')[0] != 'application/json':
            # forces a CORS preflight, which this server never answers, so other web pages cannot start a run
            self._send(415, {'error': 'Content-Type must be application/json'})
            return
        try:
            body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or b'{}')
            i, k = body['item'], body['cmd']
            assert isinstance(i, int) and isinstance(k, int) and not isinstance(i, bool) and not isinstance(k, bool)
            c = parse(HANDOVER)[i]['cmds'][k]['cmd']
        except (ValueError, KeyError, TypeError, IndexError, AssertionError):
            self._send(400, {'error': 'bad item or cmd index'})
            return
        with LOCK:
            if proc is not None and run_exit is None:
                self._send(409, {'busy': cmd})
                return
            proc = subprocess.Popen(shlex.split(c), cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    text=True, bufsize=1, start_new_session=True)
            item, cmd_index, cmd = i, k, c
            started, run_exit, lines = time.time(), None, []
            threading.Thread(target=reader, args=(proc, c), daemon=True).start()
        self._send(200, {'ok': True})


def main():
    global HANDOVER, last
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--port', type=int, default=8765)
    ap.add_argument('--handover', default='docs/handover.md')
    ap.add_argument('--open', action='store_true')
    ap.add_argument('--parse', action='store_true')
    args = ap.parse_args()
    HANDOVER = ROOT / args.handover
    if args.parse:
        print(json.dumps(parse(HANDOVER), indent=1))
        return 0
    if STATE.exists():
        try:
            last = json.loads(STATE.read_text())
        except ValueError:
            last = {}
    httpd = ThreadingHTTPServer(('127.0.0.1', args.port), Handler)
    url = f'http://127.0.0.1:{args.port}'
    print(f'checks: {url}', flush=True)
    if args.open:
        webbrowser.open(url)
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
