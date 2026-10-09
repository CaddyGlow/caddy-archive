import argparse
import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "compatibility", Path(__file__).resolve().parents[1] / "check-archive-compatibility.py")
compatibility = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(compatibility)

FAKE = '''#!/usr/bin/env python3
import pathlib, sys, zipfile
args = sys.argv[1:]
if args == ['i']:
    print('7-Zip [64] VERSION : fixture build')
elif args == ['--version']:
    print('arc test fixture')
elif args[0] in ('a', 'create'):
    output = args[args.index('--output') + 1] if args[0] == 'create' else args[-2]
    source = pathlib.Path(args[args.index('--input') + 1]) if args[0] == 'create' else pathlib.Path('.')
    with zipfile.ZipFile(output, 'w') as archive:
        for path in source.rglob('*'):
            if path.is_file(): archive.write(path, str(path.relative_to(source)))
elif args[0] in ('x', 'extract'):
    source = args[1] if args[0] == 'extract' else args[2]
    output = args[args.index('--output') + 1] if args[0] == 'extract' else args[3][2:]
    with zipfile.ZipFile(source) as archive: archive.extractall(output)
elif args[0] in ('rn', 'rename', 'd', 'delete'):
    source = args[1]
    rename = args[0] in ('rn', 'rename')
    at = args.index('--pair') + 1 if args[0] == 'rename' else args.index('--name') + 1 if args[0] == 'delete' else 2
    old = args[at]
    new = args[at + 1] if rename else None
    with zipfile.ZipFile(source) as archive:
        entries = [(name, archive.read(name)) for name in archive.namelist()]
        comment = archive.comment
    with zipfile.ZipFile(source, 'w') as archive:
        archive.comment = comment
        for name, data in entries:
            matches = name == old or old.endswith('/') and name.startswith(old)
            if matches and not rename: continue
            if matches: name = new + name[len(old):]
            archive.writestr(name, data)
elif args[0] in ('l', 'list', 't', 'test'):
    with zipfile.ZipFile(args[1]) as archive:
        assert archive.testzip() is None
        print(archive.namelist())
else:
    sys.exit(3)
'''


class CompatibilityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def args(self, version='26.04', allow=False):
        tool = self.root / 'fake tool'
        tool.write_text(FAKE.replace('VERSION', version))
        tool.chmod(0o755)
        return argparse.Namespace(reference=str(tool), arc=str(tool), timeout=3,
                                  allow_reference_mismatch=allow, profile=['zip-copy'])

    def test_pinned_payload_roundtrips_and_evidence(self):
        report, code = compatibility.check(self.args(), self.root)
        self.assertEqual(code, 0)
        self.assertTrue(report['pinned_conformance'])
        self.assertEqual(len(report['probes'][0]['extractions']), 4)
        self.assertIn('sha256', report['reference'])
        self.assertIn('fixture build', report['reference']['build_configuration']['stdout'])
        self.assertEqual(len(report['probes'][0]['fixtures']['sha256']), 3)

    def test_wrong_pin_refuses_probes(self):
        report, code = compatibility.check(self.args('17.05'), self.root)
        self.assertEqual(code, 2)
        self.assertEqual(report['status'], 'reference-pin-mismatch')
        self.assertFalse(report['pinned_conformance'])
        self.assertEqual(report['probes'], [])

    def test_allowed_wrong_pin_never_passes(self):
        report, code = compatibility.check(self.args('17.05', True), self.root)
        self.assertEqual(code, 2)
        self.assertEqual(report['status'], 'unpinned-evidence')
        self.assertTrue(report['probes'][0]['passed'])
        self.assertFalse(report['pinned_conformance'])

    def test_opt_in_edit_probes_compare_payloads_and_comments(self):
        args = self.args()
        args.edit_probes = True
        report, code = compatibility.check(args, self.root)
        self.assertEqual(code, 0)
        self.assertEqual(len(report['probes']), 5)
        for probe in report['probes'][1:]:
            self.assertTrue(probe['passed'])
            self.assertEqual(len(probe['extractions']), 4)
            self.assertTrue(all(item['preserved'] for item in probe['comments']))

    def test_missing_reference_is_not_a_pass(self):
        args = self.args()
        args.reference = str(self.root / 'missing')
        report, code = compatibility.check(args, self.root)
        self.assertEqual(code, 2)
        self.assertEqual(report['status'], 'reference-unavailable')

    def test_official_z_banner_and_declared_provenance(self):
        args = self.args()
        tool = Path(args.reference)
        tool.write_text(tool.read_text().replace('7-Zip [64]', '7-Zip (z)'))
        args.reference_source_commit = compatibility.COMMIT
        report, code = compatibility.check(args, self.root)
        self.assertEqual(code, 0)
        self.assertEqual(report['reference']['source_provenance']['declared_commit'], compatibility.COMMIT)

    def test_unrelated_version_does_not_satisfy_pin(self):
        args = self.args()
        tool = Path(args.reference)
        tool.write_text(tool.read_text().replace('7-Zip [64]', 'unrelated tool'))
        report, code = compatibility.check(args, self.root)
        self.assertEqual(code, 2)
        self.assertIsNone(report['reference']['version'])

    def test_timeout_records_failure(self):
        tool = self.root / 'sleep'
        tool.write_text('#!/usr/bin/env python3\nimport time\ntime.sleep(5)\n')
        tool.chmod(0o755)
        result = compatibility.run([str(tool)], self.root, 0.02)
        self.assertTrue(result['timed_out'])
        self.assertIsNone(result['exit_code'])

    def test_reference_password_argument_is_redacted(self):
        args = self.args()
        result = compatibility.run([args.reference, '-pfixture-secret'], self.root, 3)
        self.assertEqual(result['argv'][1], '-p<redacted>')
        self.assertNotIn('fixture-secret', str(result))

    def test_payload_mismatch_fails(self):
        args = self.args()
        arc = self.root / 'bad arc'
        arc.write_text(FAKE.replace('VERSION', '26.04').replace(
            "archive.extractall(output)", "archive.extractall(output); pathlib.Path(output, 'extra').write_bytes(b'bad')"))
        arc.chmod(0o755)
        args.arc = str(arc)
        report, code = compatibility.check(args, self.root)
        self.assertEqual(code, 1)
        self.assertFalse(report['pinned_conformance'])


if __name__ == '__main__':
    unittest.main()
