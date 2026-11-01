"""A7: an existing runtime tag must identify the checked SHA, never silently accept another candidate."""
import importlib.util
import os
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('tag_runtime', Path(__file__).parents[1] / 'scripts/tag_runtime.py')
tag_runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tag_runtime)


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
                        return subprocess.CompletedProcess(argv, 0, existing_sha + '\trefs/tags/runtime-v0.1.0\n', '')
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


if __name__ == '__main__':
    unittest.main()
