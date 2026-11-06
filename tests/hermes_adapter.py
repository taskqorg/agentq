"""Native Hermes transform contract; core compression is checked by tests/compression.rs."""
import json
from pathlib import Path
import subprocess
import types
import unittest
from unittest.mock import patch

source=(Path(__file__).resolve().parents[1]/'integrations/hermes/__init__.py').read_text().replace('@AGENTQ_EXE@',repr('/fixture/agentq'))
adapter=types.ModuleType('hermes_adapter');exec(compile(source,'hermes-adapter','exec'),adapter.__dict__)

class HermesAdapter(unittest.TestCase):
    # Chain: agentq.md M6.11, M6.2. Risk: the hooks pass prompt, response or reason text to the binary, or a failed hook breaks Hermes.
    def test_call_and_turn_hooks_pass_metadata_only_and_never_raise(self):
        secret=dict(request_messages='SECRET',response='SECRET',assistant_message='SECRET',conversation_history=['SECRET'],reason='SECRET',user_message='SECRET')
        with patch.object(adapter.subprocess,'run') as run:
            adapter.account(api_request_id='r',session_id='s',turn_id='t',model='m',usage=dict(input_tokens=1,reasoning_tokens=2,other='SECRET'),ended_at=1.5,**secret)
            args,kw=run.call_args;self.assertEqual(args[0],['/fixture/agentq','hook','hermes','post_api_request'])
            sent=json.loads(kw['input']);self.assertEqual((sent['usage']['input_tokens'],sent['usage']['reasoning_tokens']),(1,2));self.assertNotIn('SECRET',kw['input'])
            adapter.turn_end(session_id='s',completed=False,interrupted=True,**secret)
            args,kw=run.call_args;self.assertEqual(args[0],['/fixture/agentq','hook','hermes','on_session_end'])
            self.assertEqual(json.loads(kw['input']),dict(session_id='s',completed=False,interrupted=True))
        with patch.object(adapter.subprocess,'run',side_effect=OSError()):
            adapter.account(api_request_id='r');adapter.turn_end(session_id='s')
    def test_plugin_registers_every_hook(self):
        names=[]
        adapter.register(types.SimpleNamespace(register_hook=lambda name,_:names.append(name)))
        self.assertEqual(names,['post_api_request','on_session_end'])

if __name__=='__main__':unittest.main()
