#!/usr/bin/env python3
"""Update shared CoreQ and runtime pins in the repository at the current directory (specq release contract).

AgentQ moves to the newest CoreQ tag and bumps its runtime version, which publishes a runtime tag. A consumer moves
to the newest runtime tag and to the CoreQ tag that runtime resolves, so it keeps one CoreQ source. Prints the new
pin name when a manifest changed, and nothing otherwise.
"""
import re
import subprocess
import sys
import tomllib
from pathlib import Path

CRATES = 'https://github.com/taskqorg/'
DEP = re.compile(r'^(coreq|coreq-instrument|agentq-runtime)(\s*=\s*\{[^}\n]*\})', re.M)


def run(*argv):
    return subprocess.run(argv, check=True, capture_output=True, text=True).stdout


def key(tag):
    return tuple(int(n) for n in re.search(r'(\d+)\.(\d+)\.(\d+)$', tag).groups())


def newest(repo, prefix):
    tags = re.findall(r'refs/tags/(%s\d+\.\d+\.\d+)$' % prefix, run('git', 'ls-remote', '--tags', CRATES + repo), re.M)
    return max(tags, key=key)


def commit(repo, tag):
    out = run('git', 'ls-remote', CRATES + repo, f'refs/tags/{tag}', f'refs/tags/{tag}^{{}}')
    return out.splitlines()[-1].split()[0]


def runtime_coreq(tag):
    url = f'https://raw.githubusercontent.com/taskqorg/agentq/{tag}/runtime/Cargo.toml'
    return tomllib.loads(run('curl', '-fsSL', url))['dependencies']['coreq']['tag']


def repin(text, coreq, runtime=None, runtime_rev=None):
    """Keep a dependency's other fields. CoreQ is always a tag, so it stays one source with the runtime's own pin."""
    def replace(match):
        name, body = match.groups()
        if name == 'agentq-runtime':
            if runtime is None:
                return match.group(0)
            pin = f'rev = "{runtime_rev}"' if re.search(r'\brev\s*=', body) else f'tag = "{runtime}"'
        else:
            pin = f'tag = "{coreq}"'
        return name + re.sub(r'\b(?:tag|rev)\s*=\s*"[^"]*"', pin, body)
    return DEP.sub(replace, text)


def bump_patch(text):
    return re.sub(r'^version = "(\d+\.\d+\.)(\d+)"', lambda m: f'version = "{m[1]}{int(m[2]) + 1}"', text, count=1, flags=re.M)


def main():
    manifests = [Path(p) for p in run('git', 'ls-files', '*Cargo.toml').split()]
    agentq = Path('runtime/Cargo.toml') in manifests
    if agentq:
        manifests = [Path('Cargo.toml'), Path('runtime/Cargo.toml')]
        name = coreq = newest('coreq', 'v')
        runtime = runtime_rev = None
    else:
        name = runtime = newest('agentq', 'runtime-v')
        coreq = runtime_coreq(runtime)
        runtime_rev = commit('agentq', runtime)
    changed = False
    for path in manifests:
        old = path.read_text(encoding='utf-8')
        new = repin(old, coreq, runtime, runtime_rev)
        if new != old:
            path.write_text(new, encoding='utf-8')
            changed = True
    if not changed:
        return 0
    if agentq:
        path = Path('runtime/Cargo.toml')
        text = bump_patch(path.read_text(encoding='utf-8'))
        path.write_text(text, encoding='utf-8')
        version = tomllib.loads(text)['package']['version']
        log = Path('runtime/CHANGELOG.md')
        entry = f'# Runtime changelog\n\n## {version}\n\n- Pin CoreQ {coreq}.\n'
        log.write_text(log.read_text(encoding='utf-8').replace('# Runtime changelog\n', entry, 1), encoding='utf-8')
    # Resolving without --locked moves only what the edited manifests require.
    run('cargo', 'metadata', '--format-version', '1')
    print(name)
    return 0


if __name__ == '__main__':
    sys.exit(main())
