import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import release_provenance as provenance
SPEC = importlib.util.spec_from_file_location('source_release', SCRIPTS / 'source-release.py')
source_release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(source_release)


def git(root, *arguments):
    subprocess.run(['git', '-C', str(root), *arguments], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


class ReleaseSourcesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.parent = Path(self.temporary.name)
        self.root = self.parent / 'repo'
        self.root.mkdir()
        (self.root / '.github').mkdir()
        record = json.loads((SCRIPTS.parent / provenance.PROVENANCE).read_text())
        (self.root / provenance.PROVENANCE).write_text(json.dumps(record))
        (self.root / 'Cargo.toml').write_text('[workspace]\nmembers = []\n[workspace.package]\nversion = "0.2.1"\n')
        entries = ['version = 4\n', '[[package]]\nname = "caddy-archive-core"\nversion = "0.2.1"\n']
        for name, package in record['packages'].items():
            entries.append(f'[[package]]\nname = "{name}"\nversion = "{package["version"]}"\nsource = "{record["registry_source"]}"\nchecksum = "{package["checksum"]}"\n')
        (self.root / 'fuzz').mkdir()
        for relative in provenance.LOCKS:
            (self.root / relative).parent.mkdir(parents=True, exist_ok=True)
            (self.root / relative).write_text('\n'.join(entries))
        (self.root / 'LICENSE').write_text('Preserved fixture license\n')
        git(self.root, 'init', '-q')
        git(self.root, 'add', '.')
        git(self.root, '-c', 'user.name=Release test', '-c', 'user.email=release-test@example.invalid', 'commit', '-qm', 'fixture')

    def test_registry_bridge_identity_is_allowed_but_path_override_and_stale_lock_fail(self):
        provenance.validate_locks(self.root)
        lock = self.root / 'fuzz/Cargo.lock'
        original = lock.read_text()
        lock.write_text(original.replace('version = "0.2.2"', 'version = "0.2.1"'))
        with self.assertRaisesRegex(ValueError, 'differs from qualified'):
            provenance.validate_locks(self.root)
        lock.write_text(original + '\n[[package]]\nname = "ms-package"\nversion = "0.2.2"\n')
        with self.assertRaisesRegex(ValueError, 'non-registry'):
            provenance.validate_locks(self.root)

    def test_moving_source_reference_is_rejected(self):
        path = self.root / provenance.PROVENANCE
        record = json.loads(path.read_text())
        record['packages']['ms-package']['git_commit'] = 'main'
        path.write_text(json.dumps(record))
        with self.assertRaisesRegex(ValueError, 'immutable'):
            provenance.load(self.root)

    def test_identical_clean_sources_produce_identical_archives_and_manifest(self):
        first, second = self.parent / 'first.tar.gz', self.parent / 'second.tar.gz'
        self.assertEqual(source_release.create(self.root, first), source_release.create(self.root, second))
        self.assertEqual(first.read_bytes(), second.read_bytes())
        verified = source_release.verify(first)
        self.assertFalse(verified['preview'])
        self.assertEqual(verified['locks_sha256'], provenance.validate_locks(self.root))
        self.assertIn('archive-rs/LICENSE', verified['files'])

    def test_dirty_source_requires_explicit_preview_and_preserves_untracked_evidence(self):
        (self.root / 'new-evidence.txt').write_text('regression evidence\n')
        output = self.parent / 'preview.tar.gz'
        with self.assertRaisesRegex(ValueError, 'clean committed'):
            source_release.create(self.root, output)
        source_release.create(self.root, output, allow_dirty=True)
        verified = source_release.verify(output)
        self.assertTrue(verified['preview'])
        self.assertTrue(verified['source']['dirty'])
        self.assertIn('archive-rs/new-evidence.txt', verified['files'])
        with self.assertRaisesRegex(ValueError, 'overwrite'):
            source_release.create(self.root, output, allow_dirty=True)

    def test_extracted_file_corruption_is_detected(self):
        original = self.parent / 'original.tar.gz'
        source_release.create(self.root, original)
        extracted = self.parent / 'extracted'
        extracted.mkdir()
        with tarfile.open(original) as archive:
            archive.extractall(extracted, filter='data')
        bundle = next(extracted.iterdir())
        (bundle / 'archive-rs/LICENSE').write_text('changed')
        modified = self.parent / 'modified.tar.gz'
        with tarfile.open(modified, 'w:gz') as archive:
            archive.add(bundle, arcname=bundle.name)
        with self.assertRaisesRegex(ValueError, 'file manifest'):
            source_release.verify(modified)

    def test_output_inside_checkout_and_wrong_tag_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'outside'):
            source_release.create(self.root, self.root / 'source.tar.gz')
        with self.assertRaisesRegex(ValueError, 'version'):
            source_release.create(self.root, self.parent / 'source.tar.gz', tag='v9.0.0')


if __name__ == '__main__':
    unittest.main()
