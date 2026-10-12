#!/usr/bin/env python3
"""Installer modes and preservation of mail settings; local fixtures, no downloads."""
import hashlib
import os
from pathlib import Path
import stat
import subprocess
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='codex24h-install-test-')
        self.addCleanup(self.directory.cleanup)
        self.base = Path(self.directory.name)
        self.home = self.base/'home'
        self.bin = self.home/'.local/bin'
        self.lib = self.home/'.local/lib/codex24h'
        self.files = {
            '.config/codex24h/mail.toml': b'enabled = true\npassword_file = "smtp-password"\n',
            '.config/codex24h/smtp-password': b'fixture-password\n',
            '.local/state/codex24h/mail/deliveries.sqlite': b'fixture-queue',
            '.codex/config.toml': b'notify = ["existing-notifier"]\n',
            'custom/mail.toml': b'enabled = true\n',
        }
        for name, contents in self.files.items():
            path = self.home/name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(contents)
            path.chmod(0o600)
        self.env = os.environ.copy()
        for key in ('CODEX24H_VERSION', 'CODEX24H_BIN_DIR', 'CODEX24H_LIB_DIR'):
            self.env.pop(key, None)
        self.env.update(HOME=str(self.home), XDG_CONFIG_HOME=str(self.home/'.config'),
                        XDG_STATE_HOME=str(self.home/'.local/state'),
                        CODEX24H_MAIL_CONFIG=str(self.home/'custom/mail.toml'))

    def archive(self, version):
        package = self.base/version
        package.mkdir()
        (package/'VERSION').write_text(version+'\n')
        for name in ('codex24h', 'codex24h-mail', 'codex24h-attach', 'codex24h-requests', 'codex24h-session'):
            path = package/name
            path.write_text('#!/bin/sh\nexit 0\n')
            path.chmod(0o755)
        archive = self.base/'codex24h-linux-x86_64.tar.gz'
        with tarfile.open(archive, 'w:gz') as tar:
            for path in package.iterdir():
                tar.add(path, arcname=path.name)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        Path(str(archive)+'.sha256').write_text(digest+'  '+archive.name+'\n')
        self.env['CODEX24H_ARCHIVE'] = str(archive)

    def install(self):
        return subprocess.run(['bash', str(ROOT/'install.sh')], env=self.env,
                              text=True, capture_output=True, timeout=15)

    def assert_preserved(self):
        for name, contents in self.files.items():
            path = self.home/name
            self.assertEqual(path.read_bytes(), contents, name)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600, name)

    def test_first_install_same_version_and_update_preserve_mail(self):
        self.archive('9.9.1')
        first = self.install()
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertIn('first installation', first.stdout)
        self.assert_preserved()
        same = self.install()
        self.assertEqual(same.returncode, 0, same.stderr)
        self.assertIn('Already installed codex24h v9.9.1', same.stdout)
        self.assert_preserved()
        self.archive('9.9.2')
        update = self.install()
        self.assertEqual(update.returncode, 0, update.stderr)
        self.assertIn('Updated codex24h v9.9.1 -> v9.9.2', update.stdout)
        self.assertEqual((self.lib/'current').resolve().name, 'v9.9.2')
        self.assert_preserved()

    def test_legacy_source_install_is_an_update(self):
        self.bin.mkdir(parents=True)
        (self.bin/'codex24h').write_text('old source installation')
        (self.bin/'codex24h-mail').write_text('old mail helper')
        self.archive('9.9.1')
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('Updated existing codex24h installation to v9.9.1', result.stdout)
        self.assertTrue((self.bin/'codex24h-mail').is_symlink())
        self.assert_preserved()

    def test_failed_update_keeps_installed_version_and_mail(self):
        self.archive('9.9.1')
        self.assertEqual(self.install().returncode, 0)
        self.archive('9.9.2')
        Path(self.env['CODEX24H_ARCHIVE']).write_bytes(b'corrupt download')
        failed = self.install()
        self.assertNotEqual(failed.returncode, 0)
        self.assertNotIn('Updated', failed.stdout)
        self.assertEqual((self.lib/'current').resolve().name, 'v9.9.1')
        self.assert_preserved()


if __name__ == '__main__':
    unittest.main()
