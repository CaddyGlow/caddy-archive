#!/usr/bin/env python3
"""Create and verify deterministic standalone sources with registry provenance."""
import argparse
import gzip
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
from release_provenance import load, sha256, validate_locks


def run(command, **kwargs):
    return subprocess.run(command, check=True, text=True, **kwargs)


def normalize(info):
    info.mtime = 0
    info.uid = info.gid = 0
    info.uname = info.gname = ''
    info.mode = 0o755 if info.isdir() or info.mode & 0o111 else 0o644
    info.pax_headers = {}
    return info


def file_hashes(root):
    result = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError(f'Source symlink requires review: {path.relative_to(root)}')
        if path.is_file() and path != root / 'SOURCE-MANIFEST.json':
            result[str(path.relative_to(root))] = sha256(path)
    return result


def create(root, output, allow_dirty=False, tag=None):
    root = root.resolve()
    record = load(root)
    locks = validate_locks(root, record)
    version = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version']
    if tag and tag != f'v{version}':
        raise ValueError(f'Release tag must match workspace version v{version}')
    status = run(['git', '-C', str(root), 'status', '--porcelain'], capture_output=True).stdout
    if status and not allow_dirty:
        raise ValueError('Official source packaging requires a clean committed tree; use --allow-dirty only for a preview')
    commit = run(['git', '-C', str(root), 'rev-parse', 'HEAD'], capture_output=True).stdout.strip()
    output = output.resolve()
    # Creating output inside the input tree would make a preview self-referential.
    if output.is_relative_to(root):
        raise ValueError('Source archive output must be outside the input checkout')
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        raise ValueError(f'Refusing to overwrite {output}')
    with tempfile.TemporaryDirectory(prefix='archive-source-') as temporary:
        staged = Path(temporary) / f'archive-rs-v{version}-source'
        staged.mkdir()
        archive_root = staged / 'archive-rs'
        archive_root.mkdir()
        if allow_dirty:
            paths = run(['git', '-C', str(root), 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], capture_output=True).stdout
            for relative in sorted(set(paths.split('\0')) - {''}):
                source = root / relative
                if source.is_symlink():
                    raise ValueError(f'Source symlink requires review: {relative}')
                if not source.is_file():
                    continue
                destination = archive_root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, destination)
        else:
            snapshot = Path(temporary) / 'snapshot.tar'
            run(['git', '-C', str(root), 'archive', '--format=tar', f'--output={snapshot}', commit])
            with tarfile.open(snapshot) as archive:
                archive.extractall(archive_root, filter='data')
        # Verify the actual staged inputs, not merely the caller's working tree.
        staged_record = load(archive_root)
        staged_locks = validate_locks(archive_root, staged_record)
        if staged_record != record or staged_locks != locks:
            raise ValueError('Source inputs changed during packaging')
        (staged / 'COMMITS').write_text(f'archive-rs {commit}\n')
        manifest = {
            'schema_version': 1, 'version': version, 'preview': bool(allow_dirty),
            'source': {'git_commit': commit, 'dirty': bool(status)},
            'dependency_provenance': record, 'locks_sha256': locks,
            'files': file_hashes(staged),
            'qualification': 'Standalone sources require crates.io access or a populated Cargo cache; upstream authoring test sources are separately pinned.',
        }
        (staged / 'SOURCE-MANIFEST.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
        # Exclusive creation preserves an existing result even if another writer
        # races the earlier existence check.
        with output.open('xb') as raw:
            with gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode='w', format=tarfile.PAX_FORMAT) as archive:
                    archive.add(staged, arcname=staged.name, filter=normalize)
    return sha256(output)


def verify(archive_path, build=False):
    with tempfile.TemporaryDirectory(prefix='archive-source-extracted-') as temporary:
        destination = Path(temporary)
        with tarfile.open(archive_path) as archive:
            members = archive.getmembers()
            if any(not (member.isfile() or member.isdir()) for member in members):
                raise ValueError('Source archives must contain regular files/directories only')
            archive.extractall(destination, filter='data')
        roots = list(destination.iterdir())
        if len(roots) != 1 or not roots[0].is_dir():
            raise ValueError('Expected exactly one source root')
        root = roots[0]
        recorded = json.loads((root / 'SOURCE-MANIFEST.json').read_text())
        if recorded.get('schema_version') != 1 or file_hashes(root) != recorded['files']:
            raise ValueError('Source file manifest verification failed')
        archive_root = root / 'archive-rs'
        provenance = load(archive_root)
        if recorded['dependency_provenance'] != provenance or recorded['locks_sha256'] != validate_locks(archive_root, provenance):
            raise ValueError('Source package dependency provenance differs')
        if build:
            run(['cargo', 'test', '--workspace', '--all-features', '--locked'], cwd=archive_root)
            run(['cargo', 'test', '--manifest-path', 'fuzz/Cargo.toml', '--locked'], cwd=archive_root)
        return recorded


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--output', type=Path)
    parser.add_argument('--tag')
    parser.add_argument('--allow-dirty', action='store_true', help='Explicitly mark an uncommitted local preview')
    parser.add_argument('--verify', type=Path)
    parser.add_argument('--build', action='store_true', help='Run workspace and fuzz locked tests from extracted sources')
    args = parser.parse_args()
    if args.verify:
        result = verify(args.verify, args.build)
        print(json.dumps({'source': result['source'], 'preview': result['preview'], 'build_verified': args.build}, sort_keys=True))
    elif args.output:
        print(f'{create(args.root, args.output, args.allow_dirty, args.tag)}  {args.output.name}')
    else:
        parser.error('supply --output or --verify')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError) as error:
        raise SystemExit(str(error)) from error
