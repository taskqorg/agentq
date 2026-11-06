"""Shared pin updates keep one CoreQ source and each dependency's other fields."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('pin_update', Path(__file__).parents[1] / 'scripts/pin_update.py')
pin_update = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pin_update)
GIT = 'git = "https://github.com/taskqorg/'


class Repin(unittest.TestCase):
    def test_consumer_moves_runtime_and_coreq_together(self):
        old = f'agentq-runtime = {{ {GIT}agentq", tag = "runtime-v0.5.0" }}\ncoreq = {{ {GIT}coreq", tag = "v0.19.2" }}\ncoreq-instrument = {{ {GIT}coreq", tag = "v0.19.2" }}\nserde = "1"\n'
        new = f'agentq-runtime = {{ {GIT}agentq", tag = "runtime-v0.13.1" }}\ncoreq = {{ {GIT}coreq", tag = "v0.23.0" }}\ncoreq-instrument = {{ {GIT}coreq", tag = "v0.23.0" }}\nserde = "1"\n'
        self.assertEqual(pin_update.repin(old, 'v0.23.0', 'runtime-v0.13.1', 'a' * 40), new)

    def test_rev_pin_stays_a_rev_and_coreq_becomes_a_tag(self):
        old = f'agentq-runtime = {{ {GIT}agentq", rev = "{"b" * 40}" }}\ncoreq = {{ {GIT}coreq", rev = "{"c" * 40}", default-features = false }}\n'
        new = f'agentq-runtime = {{ {GIT}agentq", rev = "{"a" * 40}" }}\ncoreq = {{ {GIT}coreq", tag = "v0.23.0", default-features = false }}\n'
        self.assertEqual(pin_update.repin(old, 'v0.23.0', 'runtime-v0.13.1', 'a' * 40), new)

    def test_agentq_leaves_its_own_crate_alone(self):
        old = 'agentq-runtime = { path = "runtime" }\n'
        self.assertEqual(pin_update.repin(old, 'v0.23.0'), old)

    def test_newest_compares_numbers_and_patch_bump(self):
        self.assertEqual(max(['runtime-v0.9.0', 'runtime-v0.13.1'], key=pin_update.key), 'runtime-v0.13.1')
        self.assertEqual(pin_update.bump_patch('[package]\nversion = "0.13.1"\n'), '[package]\nversion = "0.13.2"\n')


if __name__ == '__main__':
    unittest.main()
