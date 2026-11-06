"""A7: an existing runtime tag must identify the checked SHA, never silently accept another candidate.

#260 A1: a consumer that spells the documented CoreQ dependency beside the runtime resolves one CoreQ source.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('tag_runtime', Path(__file__).parents[1] / 'scripts/tag_runtime.py')
tag_runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tag_runtime)
# The tag follows the crate version, so a version bump keeps this fixture valid.
VERSION = tomllib.loads((Path(__file__).parents[1] / 'runtime/Cargo.toml').read_text())['package']['version']


class Publication(unittest.TestCase):
    def test_existing_tag_must_match_checked_sha(self):
        sha = 'a' * 40
        for existing_sha in [sha, 'b' * 40]:
            with self.subTest(existing_sha=existing_sha):
                def run(argv, **kwargs):
                    if argv[0] == 'curl':
                        jobs = [{'name': name, 'conclusion': 'success', 'head_sha': sha} for name in
                                ['contract', 'rust (ubuntu-latest)', 'rust (macos-latest)', 'rust (windows-latest)']]
                        return subprocess.CompletedProcess(argv, 0, tag_runtime.json.dumps({'jobs': jobs}), '')
                    if argv[:2] == ['git', 'rev-parse']:
                        return subprocess.CompletedProcess(argv, 0, sha + '\n', '')
                    if argv[:2] == ['git', 'ls-remote']:
                        return subprocess.CompletedProcess(argv, 0, existing_sha + f'\trefs/tags/runtime-v{VERSION}\n', '')
                    if argv[:2] == ['git', 'merge-base']:
                        return subprocess.CompletedProcess(argv, 0, '', '')
                    self.fail('existing immutable tags must never be rewritten: ' + repr(argv))
                with patch.dict(os.environ, CANDIDATE_SHA=sha, CHECKED_RUN='1', GITHUB_REPOSITORY='taskqorg/agentq'), \
                        patch.object(tag_runtime.subprocess, 'run', side_effect=run):
                    if existing_sha == sha:
                        tag_runtime.main()
                    else:
                        with self.assertRaises(ValueError):
                            tag_runtime.main()


class OneCoreQSource(unittest.TestCase):
    # Risk: a runtime pin apart from the consumer's makes two CoreQ crates, two `Store` types and a refused TaskQ web build.
    def test_consumer_of_runtime_and_coreq_resolves_one_source(self):
        repository = Path(__file__).resolve().parents[1]
        pin = tomllib.loads((repository / 'runtime/Cargo.toml').read_text())['dependencies']['coreq']
        self.assertEqual(pin, tomllib.loads((repository / 'Cargo.toml').read_text())['dependencies']['coreq'])
        self.assertEqual(set(pin), {'git', 'tag'}, 'the documented consumer spelling is a tag')
        with tempfile.TemporaryDirectory() as temporary:
            consumer = Path(temporary)
            (consumer / 'src').mkdir()
            (consumer / 'src/lib.rs').write_text('')
            shutil.copy(repository / 'Cargo.lock', consumer / 'Cargo.lock')
            (consumer / 'Cargo.toml').write_text(
                '[package]\nname = "consumer"\nversion = "0.0.0"\nedition = "2024"\n\n[dependencies]\n'
                f"agentq-runtime = {{ path = '{(repository / 'runtime').as_posix()}' }}\n"
                f'coreq = {{ git = "{pin["git"]}", tag = "{pin["tag"]}" }}\n\n[workspace]\n')
            metadata = subprocess.run([os.environ.get('CARGO', 'cargo'), 'metadata', '--format-version', '1', '--offline'],
                                      cwd=consumer, check=True, text=True, capture_output=True).stdout
        sources = {p['source'] for p in json.loads(metadata)['packages'] if p['name'] in ('coreq', 'coreq-instrument')}
        self.assertEqual(len(sources), 1, sources)
        self.assertIn(f'?tag={pin["tag"]}#', sources.pop())


if __name__ == '__main__':
    unittest.main()
