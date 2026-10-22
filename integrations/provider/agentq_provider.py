"""Codex tool-result adapter for Headroom's supported ASGI extension seam."""
import asyncio
import copy
import hashlib
import json
import logging
import os
from pathlib import Path
import re
import subprocess
import threading
import time

LOG = logging.getLogger('agentq.provider')
LIMIT = 64 * 1024 * 1024
FINISHED = ('response.completed', 'response.incomplete', 'response.failed')


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


def code_mode_command(source):
    """Only the literal single-tool carrier is qualified; never execute or interpret JavaScript."""
    if not isinstance(source,str): return None
    match = re.fullmatch(r"\s*(?:const|let)\s+([A-Za-z_$][\w$]*)\s*=\s*await\s+tools\.exec_command\(\s*(\{.*\})\s*\)\s*;\s*text\(\s*\1\s*\)\s*;?\s*",source,re.S)
    if not match: return None
    remaining = match[2][1:-1].strip()
    fields = {}
    decoder = json.JSONDecoder()
    while remaining:
        key = re.match(r'([A-Za-z_$][\w$]*|"(?:[^"\\]|\\.)*")\s*:\s*', remaining)
        if not key: return None
        name = json.loads(key[1]) if key[1].startswith('"') else key[1]
        if name in fields: return None
        try: value,end = decoder.raw_decode(remaining[key.end():])
        except ValueError: return None
        fields[name] = value
        remaining = remaining[key.end()+end:].strip()
        if not remaining: break
        if not remaining.startswith(','): return None
        remaining = remaining[1:].strip()
    return fields.get('cmd') if isinstance(fields.get('cmd'),str) else None


def native_text(value):
    """Select only the terminal output leaf; protocol metadata never goes to the compressor."""
    if isinstance(value, str):
        try:
            obj = json.loads(value)
        except ValueError:
            obj = None
        if isinstance(obj, dict) and isinstance(obj.get('output'), str):
            metadata = obj.get('metadata')
            code = obj.get('exit_code')
            if code is None and isinstance(metadata,dict): code=metadata.get('exit_code')
            if isinstance(code, int) and not isinstance(code, bool):
                def replace(text):
                    updated = dict(obj, output=text)
                    return json.dumps(updated, ensure_ascii=False)
                return obj['output'], code, replace
        # Native exec_command/shell wrappers carry the exit code ahead of the output.
        match = re.search(r'(?:Process exited with code|Exit code:?)\s+(-?\d+).*?\n(?:Final output:|Output:)\n', value, re.S)
        if match:
            at = match.end()
            return value[at:], int(match[1]), lambda text: value[:at] + text
    if isinstance(value, list):
        for index, part in enumerate(value):
            if isinstance(part, dict) and part.get('type') in ('input_text', 'text') and isinstance(part.get('text'), str):
                found = native_text(part['text'])
                if found:
                    text, code, replace_leaf = found
                    def replace(text, index=index, replace_leaf=replace_leaf):
                        result = copy.deepcopy(value)
                        result[index]['text'] = replace_leaf(text)
                        return result
                    return text, code, replace
    return None


class State:
    def __init__(self, install, binary):
        self.install = Path(install)
        self.binary = str(binary)
        self.root = self.install / 'data/provider'
        try:
            self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        except OSError:
            LOG.warning('provider replay storage unavailable; original will be preserved')
        self.lock = threading.RLock()
        self.calls = {}

    def observe(self, session, obj):
        if not session or len(session)>256 or not isinstance(obj, dict):
            return
        finished = obj.get('response') if obj.get('type') in FINISHED else obj if obj.get('object') == 'response' and obj.get('status') in ('completed', 'incomplete', 'failed') else None
        if isinstance(finished, dict):
            self.account(session, finished)
        item = obj.get('item', obj)
        if isinstance(item,dict) and item.get('type') == 'custom_tool_call' and item.get('name') == 'exec':
            command = code_mode_command(item.get('input'))
            if command and len(command.encode())<=65536 and isinstance(item.get('call_id'),str) and len(item['call_id'])<=256:
                with self.lock:
                    if len(self.calls)<4096: self.calls[(session,item['call_id'])]=('exec_command',command)
        if isinstance(item, dict) and item.get('type') == 'function_call':
            name = item.get('name', '')
            if not isinstance(name,str): return
            name = name.rsplit('.', 1)[-1]
            if name not in ('exec_command', 'shell'):
                return
            try:
                args = json.loads(item.get('arguments', '{}'))
                command = args.get('cmd', args.get('command'))
                if isinstance(command, list) and len(command) == 3 and command[1] in ('-c', '-lc'):
                    command = command[2]
                if isinstance(command, list) and all(isinstance(a, str) and not re.search(r'\s', a) for a in command):
                    command = ' '.join(command)
                if isinstance(command, str) and len(command.encode())<=65536 and isinstance(item.get('call_id'), str) and len(item['call_id'])<=256:
                    with self.lock:
                        if len(self.calls) < 4096:
                            self.calls[(session, item['call_id'])] = (name, command)
            except (ValueError, TypeError, AttributeError):
                return
        response = obj.get('response')
        if isinstance(response, dict) and isinstance(response.get('output'), list):
            for item in response['output']:
                self.observe(session, item)

    def account(self, session, response):
        """Model accounting: only call identity, model and token counts leave; the stream is never delayed."""
        payload = dict(session=session, **{k: response[k] for k in ('id', 'model', 'usage', 'created_at') if k in response})
        def run():
            try:
                proc = subprocess.run([self.binary, 'model', 'codex'], input=json.dumps(payload), text=True, capture_output=True,
                                      timeout=10, env=dict(os.environ, AGENTQ_INSTALL=str(self.install)))
                if proc.returncode != 0: LOG.warning('model call not recorded: %s', proc.stdout.strip()[:200])
            except (OSError, subprocess.TimeoutExpired):
                LOG.warning('model call not recorded; agentq unavailable')
        threading.Thread(target=run, daemon=True).start()

    def transform(self, session, body):
        if isinstance(body,dict) and isinstance(body.get('response'),dict):
            nested = self.transform(session, body['response'])
            return body if nested is body['response'] else dict(body, response=nested)
        if not session or len(session) > 256 or not isinstance(body, dict) or not isinstance(body.get('input'), list):
            return body
        result = copy.deepcopy(body)
        changed = False
        with self.lock:
            path = self.root / (digest(session) + '.json')
            try:
                if path.exists() and path.stat().st_size>16*1024*1024: raise ValueError('replay state too large')
                memo = json.loads(path.read_text()) if path.exists() else {}
                if not isinstance(memo, dict) or not all(isinstance(v,dict) and isinstance(v.get('input'),str) and 'output' in v and isinstance(v['output'],(str,list)) for v in memo.values()):
                    raise ValueError('invalid replay state')
            except (OSError, ValueError):
                LOG.warning('provider replay state unreadable; original preserved')
                return body
            for item in result['input']:
                if not isinstance(item, dict) or item.get('type') not in ('function_call_output','custom_tool_call_output'):
                    continue
                call = item.get('call_id')
                if not isinstance(call, str):
                    continue
                original = item.get('output')
                key = digest(call)
                fingerprint = digest(original)
                if key in memo:
                    saved = memo[key]
                    if saved.get('input') == fingerprint:
                        item['output'] = saved['output']
                        changed |= item['output'] != original
                    # An already-transmitted call is never recompressed with changed bytes.
                    continue
                known = self.calls.get((session, call))
                if not known or len(memo) >= 512:
                    continue
                found = native_text(original)
                if not found:
                    continue
                text, code, replace = found
                tool, command = known
                payload = dict(adapter='provider', tool=tool, command=command, text=text,
                               exit_code=code, session=session, tool_call=call)
                candidate = original
                try:
                    env = dict(os.environ, AGENTQ_INSTALL=str(self.install))
                    proc = subprocess.run([self.binary, 'compress', 'codex'], input=json.dumps(payload),
                                          text=True, capture_output=True, timeout=4, env=env)
                    if proc.returncode == 0:
                        compressed = json.loads(proc.stdout)
                        if isinstance(compressed, dict) and isinstance(compressed.get('text'), str):
                            candidate = replace(compressed['text']) if compressed['text'] != text else original
                except (OSError, ValueError, TypeError, subprocess.TimeoutExpired):
                    LOG.warning('provider compressor unavailable; original preserved')
                memo[key] = dict(input=fingerprint, output=candidate)
                encoded = json.dumps(memo, ensure_ascii=False).encode()
                # Refuse new transforms rather than evicting a live prompt's replay bytes.
                try: used = sum(p.stat().st_size for p in self.root.glob('*.json')) - (path.stat().st_size if path.exists() else 0)
                except OSError:
                    memo.pop(key)
                    LOG.warning('provider replay capacity unknown; original preserved')
                    continue
                if len(encoded) > 16 * 1024 * 1024 or used + len(encoded) > LIMIT:
                    memo.pop(key)
                    LOG.warning('provider replay capacity reached; original preserved')
                    continue
                temp = path.with_suffix('.tmp')
                try:
                    fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
                    with os.fdopen(fd, 'wb') as file:
                        file.write(encoded)
                        file.flush()
                        os.fsync(file.fileno())
                    os.replace(temp, path)
                except OSError:
                    memo.pop(key)
                    LOG.warning('provider replay write failed; original preserved')
                    continue
                self.calls.pop((session, call), None)
                item['output'] = candidate
                changed |= candidate != original
        return result if changed else body


class Adapter:
    def __init__(self, app, state):
        self.app, self.state = app, state

    async def __call__(self, scope, receive, send):
        if scope['type'] not in ('http', 'websocket') or scope.get('path') not in ('/v1/responses', '/responses'):
            return await self.app(scope, receive, send)
        headers = {k.lower(): v for k, v in scope.get('headers', [])}
        session = headers.get(b'thread-id', headers.get(b'session-id', headers.get(b'session_id', headers.get(b'x-headroom-session-id', b'')))).decode('utf-8', 'replace')
        if not session:
            return await self.app(scope, receive, send)
        pending = []
        if scope['type'] == 'http':
            pieces = bytearray()
            disconnected = None
            while True:
                event = await receive()
                if event['type'] != 'http.request':
                    disconnected = event
                    break
                pieces.extend(event.get('body', b''))
                if not event.get('more_body'):
                    break
            if disconnected is not None:
                if pieces: pending.append(dict(type='http.request', body=bytes(pieces), more_body=True))
                pending.append(disconnected)
            else:
                raw = bytes(pieces)
                raw = await self.transform(session, raw)
                pending.insert(0, dict(type='http.request', body=raw, more_body=False))
                scope = dict(scope, headers=[(k, str(len(raw)).encode() if k.lower() == b'content-length' else v) for k, v in scope.get('headers', [])])
        async def incoming():
            event = pending.pop(0) if pending else await receive()
            if event['type'] == 'websocket.receive':
                field = 'text' if event.get('text') is not None else 'bytes'
                raw = event.get(field)
                if isinstance(raw, (str, bytes)):
                    modified = await self.transform(session, raw.encode() if isinstance(raw, str) else raw)
                    if modified != (raw.encode() if isinstance(raw, str) else raw):
                        event = dict(event, **{field: modified.decode() if field == 'text' else modified})
            return event
        buffered = b''
        async def outgoing(event):
            nonlocal buffered
            if event['type'] == 'websocket.send':
                try:
                    self.state.observe(session, json.loads(event.get('text') or event.get('bytes') or b'{}'))
                except (ValueError, TypeError):
                    pass
            elif event['type'] == 'http.response.body':
                buffered += event.get('body', b'')
                while b'\n' in buffered:
                    line, buffered = buffered.split(b'\n', 1)
                    if line.startswith(b'data:'):
                        try:
                            self.state.observe(session, json.loads(line[5:]))
                        except (ValueError, TypeError):
                            pass
                if len(buffered) > 1024 * 1024:
                    buffered = b''
                if not event.get('more_body') and buffered:
                    try:
                        self.state.observe(session, json.loads(buffered))
                    except (ValueError, TypeError):
                        pass
            await send(event)  # Provider events, encrypted reasoning and cancellation remain byte-identical.
        await self.app(scope, incoming, outgoing)

    async def transform(self, session, raw):
        try:
            body = json.loads(raw)
            transformed = await asyncio.to_thread(self.state.transform, session, body)
            return raw if transformed is body else json.dumps(transformed, ensure_ascii=False).encode()
        except (OSError, ValueError, TypeError, AttributeError):
            LOG.warning('provider adapter could not inspect frame; original preserved')
            return raw


def install(app, config):
    root = os.environ['AGENTQ_INSTALL']
    binary = os.environ['AGENTQ_BINARY']
    app.add_middleware(Adapter, state=State(root, binary))
