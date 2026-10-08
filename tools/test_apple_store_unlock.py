"""Synthetic secret checks; no real Apple credentials or keychain operations."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from apple_store_companion import Backend

class Tests(unittest.TestCase):
    def test_unlock_in_child_environment_not_arguments_or_parent(self):
        with tempfile.TemporaryDirectory() as directory:
            backend=Backend('tools/ipatool-unlock.exe',directory,keychain_passphrase='synthetic-unlock')
            previous=os.environ.get('IPATOOL_KEYCHAIN_PASSPHRASE')
            output=subprocess.CompletedProcess([],0,b'{"success":true}\n',b'')
            with patch('apple_store_companion.subprocess.run',return_value=output) as run:
                self.assertEqual(backend.run(['auth','info']),[{'success':True}])
                argv=run.call_args.args[0]
                self.assertNotIn('synthetic-unlock',' '.join(argv))
                self.assertEqual(run.call_args.kwargs['env']['IPATOOL_KEYCHAIN_PASSPHRASE'],'synthetic-unlock')
            self.assertEqual(os.environ.get('IPATOOL_KEYCHAIN_PASSPHRASE'),previous)
    def test_unconfigured_backend_does_not_inherit_secret(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.dict(os.environ,{'IPATOOL_KEYCHAIN_PASSPHRASE':'synthetic-inherited'}):
                backend=Backend('tools/ipatool-unlock.exe',directory)
                self.assertNotIn('IPATOOL_KEYCHAIN_PASSPHRASE',backend.child_environment())
                self.assertEqual(os.environ['IPATOOL_KEYCHAIN_PASSPHRASE'],'synthetic-inherited')
if __name__=='__main__':unittest.main()
