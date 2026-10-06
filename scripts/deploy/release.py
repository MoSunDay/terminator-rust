#!/usr/bin/env python3
"""Install immutable release artifacts and restore a durable previous version."""
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import sys
import tarfile
import tempfile

BINARIES = ('terminator-rust', 'terminator-ctl', 'terminator-session')
RECEIPT = '.release.json'


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def file_manifest(directory):
    files = {}
    for path in sorted(directory.rglob('*')):
        if path.is_symlink():
            raise ValueError('release contains a symlink: ' + str(path))
        if path.is_file() and path.name != RECEIPT:
            files[str(path.relative_to(directory))] = [digest(path), path.stat().st_mode & 0o777]
    for name in BINARIES:
        entry = files.get('bin/' + name)
        if entry is None or not entry[1] & 0o111:
            raise ValueError('missing executable: ' + name)
    return files


def sync_directory(path):
    fd = os.open(str(path), os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def write_once(path, data):
    """Publish a complete, immutable journal before changing current."""
    fd, name = tempfile.mkstemp(prefix='.receipt-', dir=str(path.parent))
    try:
        with os.fdopen(fd, 'w') as out:
            json.dump(data, out, sort_keys=True)
            out.write('\n')
            out.flush()
            os.fsync(out.fileno())
        os.link(name, str(path))  # Refuse to overwrite an earlier rollback anchor.
        sync_directory(path.parent)
    finally:
        os.unlink(name)


def read_json(path):
    return json.loads(path.read_text())


def current_target(root):
    current = root / 'current'
    if not current.is_symlink():
        if current.exists():
            raise ValueError('current must be a symlink')
        return None
    target = current.resolve(strict=True)
    if root not in target.parents:
        raise ValueError('current points outside the install root')
    return str(target.relative_to(root))


def switch_current(root, target):
    if target is None:
        (root / 'current').unlink(missing_ok=True)
    else:
        temporary = root / ('.current-' + str(os.getpid()))
        try:
            temporary.symlink_to(target)
            os.replace(str(temporary), str(root / 'current'))
        finally:
            temporary.unlink(missing_ok=True)
    sync_directory(root)


def extract_archive(archive, stage):
    """Extract a single bundle root; never follow archive links or traversal."""
    bundle = None
    seen = set()
    with tarfile.open(str(archive), 'r:gz') as package:
        for entry in package:
            path = PurePosixPath(entry.name)
            if path.is_absolute() or '..' in path.parts or not path.parts:
                raise ValueError('unsafe archive path')
            if bundle is None:
                bundle = path.parts[0]
            if path.parts[0] != bundle or not (entry.isdir() or entry.isfile()):
                raise ValueError('archive must contain one directory and regular files')
            relative = Path(*path.parts[1:])
            if str(relative) in seen or relative.name == RECEIPT:
                raise ValueError('duplicate or reserved archive path')
            seen.add(str(relative))
            target = stage / relative
            if entry.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            if relative == Path('.'):
                raise ValueError('archive root must be a directory')
            target.parent.mkdir(parents=True, exist_ok=True)
            with package.extractfile(entry) as src, target.open('xb') as out:
                shutil.copyfileobj(src, out)
                os.fchmod(out.fileno(), entry.mode & 0o777)
                out.flush()
                os.fsync(out.fileno())
    for directory in sorted((p for p in stage.rglob('*') if p.is_dir()), reverse=True):
        directory.chmod(0o755)
        sync_directory(directory)
    stage.chmod(0o755)  # mkdtemp starts at 0700; desktop users must traverse it.
    sync_directory(stage)


def verify_release(release, artifact):
    saved = read_json(release / RECEIPT)
    if saved['artifact'] != artifact or saved['files'] != file_manifest(release):
        raise ValueError('release content verification failed')


def prepare_release(root, artifact):
    release = root / 'releases' / artifact
    if release.exists():
        verify_release(release, artifact)
        return release
    archive = root / 'incoming' / (artifact + '.tar.gz')
    if digest(archive) != artifact:
        raise ValueError('archive checksum mismatch')
    stage = Path(tempfile.mkdtemp(prefix='.staging-', dir=str(root / 'releases')))
    try:
        extract_archive(archive, stage)
        write_once(stage / RECEIPT, {'artifact': artifact, 'files': file_manifest(stage)})
        os.rename(str(stage), str(release))
        sync_directory(release.parent)
    finally:
        if stage.exists():
            shutil.rmtree(stage)
    return release


def verify_previous(root, journal):
    previous = journal['previous']
    if previous is not None and file_manifest(root / previous) != journal['previous_files']:
        raise ValueError('previous release changed; refusing to restore unchecked binaries')


def install(root, artifact):
    release = prepare_release(root, artifact)
    target = str(release.relative_to(root))
    receipt = root / 'journal' / (artifact + '.json')
    current = current_target(root)
    if receipt.exists():
        journal = read_json(receipt)
        if journal['target'] != target or current not in (target, journal['previous']):
            raise ValueError('another release is active; refusing a stale activation retry')
        verify_previous(root, journal)
    else:
        if current == target:
            raise ValueError('active release has no rollback journal')
        journal = {'target': target, 'previous': current,
                   'previous_files': file_manifest(root / current) if current else None}
        write_once(receipt, journal)
    switch_current(root, target)
    if current_target(root) != target:
        raise ValueError('activation readback failed')
    return {'current': target, 'previous': journal['previous'], 'artifact': artifact}


def rollback(root, artifact):
    journal = read_json(root / 'journal' / (artifact + '.json'))
    if journal['target'] != 'releases/' + artifact:
        raise ValueError('rollback journal target mismatch')
    if journal['previous'] is None:
        raise ValueError('first install has no previous release to restore')
    current = current_target(root)
    if current not in (journal['target'], journal['previous']):
        raise ValueError('another release is active; refusing to overwrite it')
    verify_previous(root, journal)
    switch_current(root, journal['previous'])
    if current_target(root) != journal['previous']:
        raise ValueError('rollback readback failed')
    return {'current': journal['previous'], 'restored_from': artifact}


@contextlib.contextmanager
def locked_root(path):
    root = Path(path).resolve()
    if not root.exists():
        root.mkdir(parents=True)
        root.chmod(0o755)
    with (root / '.deploy.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        for directory in ('incoming', 'releases', 'journal'):
            child = root / directory
            if not child.exists():
                child.mkdir()
                child.chmod(0o755)
        yield root


def main(args):
    if len(args) not in (2, 3) or args[1] not in ('preflight', 'install', 'rollback'):
        raise ValueError('usage: release.py ROOT preflight|install|rollback [SHA256]')
    action = args[1]
    artifact = args[2] if len(args) == 3 else ''
    if action != 'preflight' and (len(artifact) != 64 or any(c not in '0123456789abcdef' for c in artifact)):
        raise ValueError('expected a full lowercase SHA256')
    with locked_root(args[0]) as root:
        if action == 'preflight':
            # Use the real release identity to prove create, fsync, rename and
            # remove capabilities without touching any effective version.
            with tempfile.TemporaryDirectory(prefix='.preflight-', dir=str(root)) as name:
                probe = Path(name)
                write_once(probe / 'write', {'probe': True})
                os.replace(str(probe / 'write'), str(probe / 'renamed'))
                sync_directory(probe)
            result = {'writable': True, 'current': current_target(root)}
        elif action == 'install':
            result = install(root, artifact)
        else:
            result = rollback(root, artifact)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    try:
        main(sys.argv[1:])
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        sys.exit('release: ' + str(error))
