#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Reads and edits assistant config files inside a test computer for
# scripts/e2e-settings.sh. Paths are dotted (`model_providers.mine.base_url`);
# files are JSON with comments, TOML (`.toml`) or dotenv (`.env`).
#
#   probe.py get  <file> <path>          print the value as JSON (exit 1: absent)
#   probe.py eq   <file> <path> <json>   exit 0 when the value equals <json>
#   probe.py set  <file> <path> <json>   set a value (rewrites JSON/TOML files)
#   probe.py del  <file> <path>          remove a value
import json
import re
import sys
import tomllib


def strip_jsonc(text):
    out, i, string = [], 0, False
    while i < len(text):
        c = text[i]
        if string:
            out.append(c)
            if c == '\\':
                out.append(text[i + 1])
                i += 1
            elif c == '"':
                string = False
        elif c == '"':
            string = True
            out.append(c)
        elif text.startswith('//', i):
            while i < len(text) and text[i] != '\n':
                i += 1
            continue
        elif text.startswith('/*', i):
            i = text.index('*/', i) + 2
            continue
        else:
            out.append(c)
        i += 1
    return re.sub(r',(\s*[}\]])', r'\1', ''.join(out))


def env_load(text):
    values = {}
    for line in text.splitlines():
        line = line.strip()
        if line.startswith('export '):
            line = line[7:]
        if '=' in line and not line.startswith('#'):
            key, value = line.split('=', 1)
            value = value.strip()
            if value[:1] in '"\'' and value:
                value = value[1:value.index(value[0], 1)]
            values[key.strip()] = value
    return values


def load(path):
    try:
        text = open(path).read()
    except FileNotFoundError:
        return {}
    if path.endswith('.toml'):
        return tomllib.loads(text)
    if path.endswith('.env'):
        return env_load(text)
    return json.loads(strip_jsonc(text)) if text.strip() else {}


def toml_dump(data, prefix=''):
    def value(v):
        if isinstance(v, bool):
            return 'true' if v else 'false'
        if isinstance(v, (int, float)):
            return str(v)
        if isinstance(v, str):
            return json.dumps(v)
        if isinstance(v, list):
            return '[' + ', '.join(value(x) for x in v) + ']'
        return '{ ' + ', '.join(f'{json.dumps(k)} = {value(x)}' for k, x in v.items()) + ' }'

    lines = [f'{json.dumps(k) if not re.match(r"^[A-Za-z0-9_-]+$", k) else k} = {value(v)}'
             for k, v in data.items() if not isinstance(v, dict)]
    for k, v in data.items():
        if isinstance(v, dict):
            name = k if re.match(r'^[A-Za-z0-9_-]+$', k) else json.dumps(k)
            lines.append(f'\n[{prefix}{name}]')
            lines.append(toml_dump(v, f'{prefix}{name}.'))
    return '\n'.join(line for line in lines if line != '')


def save(path, data):
    if path.endswith('.toml'):
        text = toml_dump(data) + '\n'
    elif path.endswith('.env'):
        text = ''.join(f'{k}={v}\n' for k, v in data.items())
    else:
        text = json.dumps(data, indent=2) + '\n'
    open(path, 'w').write(text)


def walk(data, path):
    parts = path.split('.')
    for part in parts[:-1]:
        data = data[int(part)] if isinstance(data, list) else data[part]
    return data, parts[-1]


command, file, path = sys.argv[1:4]
data = load(file)
try:
    parent, last = walk(data, path)
    present = last in parent if isinstance(parent, dict) else int(last) < len(parent)
except (KeyError, IndexError, TypeError):
    parent, last, present = None, None, False

if command == 'get':
    if not present:
        sys.exit(1)
    print(json.dumps(parent[last], sort_keys=True))
elif command == 'eq':
    sys.exit(0 if present and parent[last] == json.loads(sys.argv[4]) else 1)
elif command == 'set':
    node = data
    parts = path.split('.')
    for part in parts[:-1]:
        node = node.setdefault(part, {})
    node[parts[-1]] = json.loads(sys.argv[4])
    save(file, data)
elif command == 'del':
    if present:
        del parent[last]
        save(file, data)
else:
    sys.exit(f'unknown command {command}')
