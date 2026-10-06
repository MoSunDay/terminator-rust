#!/usr/bin/env python3
"""Exercise the real installer using private roots and subprocess crashes."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
import shutil

SCRIPT = Path(__file__).resolve().parents[2] / 'deploy' / 'release.py'
spec = importlib.util.spec_from_file_location('release', SCRIPT)
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def archive(root, text, entries=None):
    path = root / 'candidate.tar.gz'
    with tarfile.open(path, 'w:gz') as out:
        names = entries or ['bundle/bin/' + name for name in release.BINARIES]
        for name in names:
            content = ('#!/bin/sh\necho ' + text + '\n').encode()
            member = tarfile.TarInfo(name)
            member.size = len(content)
            member.mode = 0o755
            out.addfile(member, io.BytesIO(content))
    sha = hashlib.sha256(path.read_bytes()).hexdigest()
    path.rename(root / 'incoming' / (sha + '.tar.gz'))
    return sha


def call(root, action, sha=None):
    args = [sys.executable, str(SCRIPT), str(root), action]
    if sha:
        args.append(sha)
    return subprocess.run(args, capture_output=True, text=True)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='terminator-release-test-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.assertEqual(call(self.root, 'preflight').returncode, 0)
        # A legacy version directory must remain usable as a rollback anchor.
        old = self.root / 'terminator-rust-0.1.0-linux-x86_64' / 'bin'
        old.mkdir(parents=True)
        for name in release.BINARIES:
            (old / name).write_text('#!/bin/sh\necho old\n')
            (old / name).chmod(0o755)
        (self.root / 'current').symlink_to(old.parent.name)
        self.previous = old.parent.name

    def install(self, sha):
        out = call(self.root, 'install', sha)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(os.readlink(self.root / 'current'), 'releases/' + sha)
        self.assertEqual((self.root / 'current').stat().st_mode & 0o777, 0o755)
        self.assertEqual((self.root / 'current/bin').stat().st_mode & 0o777, 0o755)

    def test_same_version_artifacts_do_not_replace_each_other(self):
        a, b = archive(self.root, 'a'), archive(self.root, 'b')
        self.install(a)
        first = release.file_manifest(self.root / 'releases' / a)
        self.install(b)
        self.assertEqual(first, release.file_manifest(self.root / 'releases' / a))
        self.assertEqual(call(self.root, 'rollback', b).returncode, 0)
        self.assertEqual(os.readlink(self.root / 'current'), 'releases/' + a)
        self.assertEqual(call(self.root, 'rollback', a).returncode, 0)
        self.assertEqual(os.readlink(self.root / 'current'), self.previous)
        self.assertEqual(subprocess.check_output([str(self.root/'current/bin/terminator-rust')], text=True), 'old\n')

    def test_crash_before_and_after_activation_preserves_the_first_journal(self):
        for after in [False, True]:
            sha = archive(self.root, 'after' if after else 'before')
            code = '''import importlib.util, os, sys
spec=importlib.util.spec_from_file_location('release',sys.argv[1])
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
original=m.switch_current
def crash(root,target):
    if sys.argv[4]=='after': original(root,target)
    os._exit(81)
m.switch_current=crash
m.main([sys.argv[2],'install',sys.argv[3]])
'''
            out = subprocess.run([sys.executable, '-c', code, str(SCRIPT), str(self.root), sha,
                                  'after' if after else 'before'])
            self.assertEqual(out.returncode, 81)
            receipt = self.root / 'journal' / (sha + '.json')
            original = receipt.read_bytes()
            self.install(sha)
            self.install(sha)
            self.assertEqual(original, receipt.read_bytes())
            self.assertEqual(json.loads(original)['previous'], self.previous)
            self.assertEqual(call(self.root, 'rollback', sha).returncode, 0)
            self.assertEqual(call(self.root, 'rollback', sha).returncode, 0)
            self.assertEqual(os.readlink(self.root / 'current'), self.previous)

    def test_stale_rollback_cannot_replace_a_newer_release(self):
        a, b = archive(self.root, 'a'), archive(self.root, 'b')
        self.install(a)
        self.install(b)
        self.assertNotEqual(call(self.root, 'rollback', a).returncode, 0)
        self.assertNotEqual(call(self.root, 'install', a).returncode, 0)
        self.assertEqual(os.readlink(self.root / 'current'), 'releases/' + b)

    def test_corrupt_archive_or_installed_files_fail_before_activation(self):
        sha = archive(self.root, 'a')
        path = self.root / 'incoming' / (sha + '.tar.gz')
        saved = path.read_bytes()
        path.write_bytes(b'broken')
        self.assertNotEqual(call(self.root, 'install', sha).returncode, 0)
        self.assertEqual(os.readlink(self.root / 'current'), self.previous)
        path.write_bytes(saved)
        self.install(sha)
        (self.root / 'current/bin/terminator-rust').write_text('tampered')
        self.assertNotEqual(call(self.root, 'install', sha).returncode, 0)
        self.assertEqual(call(self.root, 'rollback', sha).returncode, 0)
        self.assertEqual(os.readlink(self.root / 'current'), self.previous)

    def test_changed_previous_release_is_not_restored(self):
        sha = archive(self.root, 'a')
        self.install(sha)
        (self.root / self.previous / 'bin/terminator-rust').write_text('tampered')
        self.assertNotEqual(call(self.root, 'rollback', sha).returncode, 0)
        self.assertEqual(os.readlink(self.root / 'current'), 'releases/' + sha)

    def test_invalid_bundle_does_not_switch_current(self):
        for paths in [['../escape'], ['bundle/bin/terminator-rust'], ['/absolute']]:
            sha = archive(self.root, 'bad', paths)
            self.assertNotEqual(call(self.root, 'install', sha).returncode, 0)
            self.assertEqual(os.readlink(self.root / 'current'), self.previous)

    def test_first_install_rollback_refuses_without_disabling_the_app(self):
        (self.root / 'current').unlink()
        sha = archive(self.root, 'first')
        self.install(sha)
        self.assertNotEqual(call(self.root, 'rollback', sha).returncode, 0)
        self.assertEqual(os.readlink(self.root / 'current'), 'releases/' + sha)

    def test_failed_archive_acceptance_prevents_remote_install_and_upload(self):
        # Exercise the actual wrapper with isolated ssh/scp executables; a
        # failed UI gate must stop before any production-mutating stage.
        checkout = self.root / 'checkout'
        scripts = checkout / 'scripts'
        (scripts / 'bin').mkdir(parents=True)
        (scripts / 'deploy').mkdir()
        shutil.copyfile(SCRIPT.parents[1] / 'bin/deploy-remote.sh', scripts / 'bin/deploy-remote.sh')
        (checkout / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.1.0"\n')
        dist = checkout / 'dist'
        dist.mkdir()
        name = 'terminator-rust-0.1.0-x86_64-unknown-linux-gnu.tar.gz'
        (dist / name).write_bytes(b'candidate')
        sha = hashlib.sha256(b'candidate').hexdigest()
        (dist / (name + '.sha256')).write_text(sha + '  ' + name + '\n')
        (scripts / 'deploy/verify-release.sh').write_text('#!/bin/sh\necho UI_GATE_FAILED >&2\nexit 9\n')
        fake_bin = checkout / 'fake-bin'
        fake_bin.mkdir()
        calls = checkout / 'remote-calls'
        for tool in ['ssh', 'scp']:
            path = fake_bin / tool
            path.write_text('#!/bin/sh\nprintf "%s\\n" "' + tool + ' $*" >> "$REVIEW_CALLS"\n')
            path.chmod(0o755)
        env = dict(os.environ, PATH=str(fake_bin) + os.pathsep + os.environ['PATH'], REVIEW_CALLS=str(calls))
        out = subprocess.run(['bash', str(scripts / 'bin/deploy-remote.sh'), '--no-build',
                              '--host', 'fixture.invalid'], env=env, capture_output=True, text=True)
        self.assertNotEqual(out.returncode, 0)
        self.assertIn('UI_GATE_FAILED', out.stderr)
        self.assertNotIn('scp ', calls.read_text())
        self.assertNotIn('python3', calls.read_text())


if __name__ == '__main__':
    unittest.main()
