"""Verify byte fixtures survive Git's Windows line-ending conversion."""

import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
FIXTURES = Path("crates/archive-wasm/tests/fixtures/package/media-migration")


class FixtureCheckoutTests(unittest.TestCase):
    def test_autocrlf_checkout_preserves_media_artifacts(self):
        manifest = json.loads((ROOT / FIXTURES / "manifest.json").read_text())
        with tempfile.TemporaryDirectory() as temporary:
            seed = Path(temporary) / "seed"
            seed.mkdir()

            def git(*arguments, cwd=seed):
                return subprocess.run(
                    ["git", *arguments], cwd=cwd, check=True,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                )

            shutil.copyfile(ROOT / ".gitattributes", seed / ".gitattributes")
            for relative in manifest["artifacts"]:
                target = seed / FIXTURES / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / FIXTURES / relative, target)
            (seed / "conversion-control.txt").write_bytes(b"first\nsecond\n")
            git("init", "--quiet")
            git("config", "user.name", "Fixture regression")
            git("config", "user.email", "fixture@example.invalid")
            git("-c", "core.autocrlf=false", "add", ".")
            git("-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "fixtures")
            checkout = Path(temporary) / "checkout"
            git("clone", "--quiet", "--no-checkout", str(seed), str(checkout))
            git("-c", "core.autocrlf=true", "checkout", "--quiet", "HEAD", cwd=checkout)
            self.assertEqual(
                (checkout / "conversion-control.txt").read_bytes(),
                b"first\r\nsecond\r\n",
                "The isolated checkout must actually apply Windows conversion",
            )
            for relative, expected in manifest["artifacts"].items():
                with self.subTest(artifact=relative):
                    actual = (checkout / FIXTURES / relative).read_bytes()
                    self.assertEqual(len(actual), expected["bytes"])
                    self.assertEqual(hashlib.sha256(actual).hexdigest(), expected["sha256"])
                    self.assertEqual(actual, (ROOT / FIXTURES / relative).read_bytes())


if __name__ == "__main__":
    unittest.main()
