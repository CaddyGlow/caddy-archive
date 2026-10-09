"""Validate registry-only package migration provenance shared by CI and bundles."""
import hashlib
import json
from pathlib import Path
import re
import tarfile
import tomllib


PROVENANCE = Path('.github/release-dependencies.json')
LOCKS = (Path('Cargo.lock'), Path('fuzz/Cargo.lock'), Path('crates/archive-wasm/tests/authoring-Cargo.lock'))


def sha256(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def load(root):
    record = json.loads((root / PROVENANCE).read_text())
    if record.get('schema_version') != 1 or not record.get('packages'):
        raise ValueError('Unsupported or empty dependency provenance')
    if record.get('registry_source') != 'registry+https://github.com/rust-lang/crates.io-index':
        raise ValueError('Release dependencies must use crates.io')
    for name, package in record['packages'].items():
        if not re.fullmatch(r'[0-9a-f]{40}', package.get('git_commit', '')):
            raise ValueError(f'{name}: expected immutable 40-character Git commit')
        if not re.fullmatch(r'[0-9a-f]{64}', package.get('checksum', '')):
            raise ValueError(f'{name}: invalid registry package SHA-256')
        if not re.fullmatch(r'\d+\.\d+\.\d+', package.get('version', '')):
            raise ValueError(f'{name}: invalid qualified package version')
    return record


def validate_locks(root, record=None):
    record = record or load(root)
    receipts = {}
    for relative in LOCKS:
        path = root / relative
        entries = tomllib.loads(path.read_text())['package']
        for name, expected in record['packages'].items():
            matching = [entry for entry in entries if entry['name'] == name]
            # The consumer's workspace core and the registry package-core bridge
            # deliberately have separate package identities, even at one version.
            registry = [entry for entry in matching if entry.get('source')]
            local = [entry for entry in matching if not entry.get('source')]
            if local and name != 'caddy-archive-core':
                raise ValueError(f'{relative}: non-registry {name} override')
            if len(registry) != 1:
                raise ValueError(f'{relative}: expected one qualified registry {name}')
            entry = registry[0]
            if any(entry.get(key) != value for key, value in (
                ('version', expected['version']), ('checksum', expected['checksum']),
                ('source', record['registry_source']),
            )):
                raise ValueError(f'{relative}: {name} differs from qualified registry version/checksum/source')
        receipts[str(relative)] = sha256(path)
    return receipts


def validate_registry_cache(registry_root, record):
    """Check cached published crate bytes and their embedded VCS provenance."""
    receipts = {}
    for name, expected in record['packages'].items():
        matches = list((registry_root / 'cache').glob(f'index.crates.io-*/{name}-{expected["version"]}.crate'))
        if len(matches) != 1:
            raise ValueError(f'{name}: expected one cached crates.io archive')
        crate = matches[0]
        if sha256(crate) != expected['checksum']:
            raise ValueError(f'{name}: registry archive checksum differs')
        with tarfile.open(crate, 'r:gz') as archive:
            vcs_path = f'{name}-{expected["version"]}/.cargo_vcs_info.json'
            stream = archive.extractfile(vcs_path)
            if stream is None:
                raise ValueError(f'{name}: packaged VCS provenance missing')
            with stream:
                vcs = json.load(stream)
        if vcs.get('git', {}).get('sha1') != expected['git_commit'] or vcs.get('path_in_vcs') != expected['path_in_vcs']:
            raise ValueError(f'{name}: packaged source revision differs')
        receipts[name] = str(crate)
    return receipts
