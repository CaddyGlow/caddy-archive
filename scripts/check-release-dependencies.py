#!/usr/bin/env python3
"""Check MSI-media dependency locks and optional cached published source archives."""
import argparse
import json
from pathlib import Path
from release_provenance import load, validate_locks, validate_registry_cache


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--registry-root', type=Path, help='Also check cached .crate SHA-256 and packaged Git provenance')
    parser.add_argument('--source-output', type=Path, help='Append qualified ms-package commit to a GitHub Actions output file')
    args = parser.parse_args()
    record = load(args.root)
    locks = validate_locks(args.root, record)
    cached = validate_registry_cache(args.registry_root, record) if args.registry_root else None
    if args.source_output:
        with args.source_output.open('a') as stream:
            stream.write(f'commit={record["packages"]["ms-package"]["git_commit"]}\n')
    print(json.dumps({'locks_sha256': locks, 'qualified_packages': record['packages'], 'checked_registry_archives': cached}, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError) as error:
        raise SystemExit(str(error)) from error
