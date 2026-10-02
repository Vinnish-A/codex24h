#!/usr/bin/env python3
"""Executable selection across installs; no actual Codex or model is started."""
import os
from pathlib import Path
import tempfile
import unittest

import e2e


class VersionSelectionTests(unittest.TestCase):
    def test_path_symlink_update_applies_to_next_launch_without_moving_running_client(self):
        with tempfile.TemporaryDirectory(prefix='codex24h-versions-') as directory:
            root = Path(directory)
            for version in ('OLD', 'NEW'):
                exe = root/version
                exe.write_text('#!/usr/bin/python3\nimport time\nprint("VERSION_'+version+'",flush=True)\ntime.sleep(30)\n')
                exe.chmod(0o755)
            link = root/'codex'
            link.symlink_to(root/'OLD')
            env = os.environ | {'PATH': str(root)+':'+os.environ['PATH'],
                                'CODEX24H_MAIL': '0', 'TERM': 'xterm-256color'}
            env.pop('CODEX24H_CODEX', None)
            old = e2e.Terminal(env)
            try:
                old.until('VERSION_OLD')
                link.unlink()
                link.symlink_to(root/'NEW')
                new = e2e.Terminal(env)
                try:
                    new.until('VERSION_NEW')
                    self.assertIsNone(old.process.poll())
                    self.assertNotIn('VERSION_NEW', e2e.visible(old.output))
                finally:
                    new.close()
                override = e2e.Terminal(env | {'CODEX24H_CODEX': str(root/'OLD')})
                try:
                    override.until('VERSION_OLD')
                finally:
                    override.close()
            finally:
                old.close()


if __name__ == '__main__':
    unittest.main()
