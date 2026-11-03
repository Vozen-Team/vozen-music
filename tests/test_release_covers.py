import hashlib
import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('release', Path(__file__).resolve().parents[1] / 'scripts/release-expansion.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class CoversRelease(unittest.TestCase):
    def test_verified_bundle_is_repeatable_and_corruption_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as folder:
            release.ROOT = Path(folder) / 'root'
            release.INCOMING = Path(folder) / 'incoming'
            release.ROOT.mkdir()
            release.INCOMING.mkdir()
            files = {}
            with tarfile.open(release.INCOMING / 'covers.tar', 'w') as bundle:
                for index in range(250):
                    data = b'synthetic artwork fixture ' + str(index).encode()
                    sha = hashlib.sha256(data).hexdigest()
                    name = sha + '.png'
                    member = tarfile.TarInfo(name)
                    member.size = len(data)
                    bundle.addfile(member, io.BytesIO(data))
                    files[name] = sha
            manifest = {'source_revision': '12345678', 'cover_files': files, 'sha256': {'covers.tar': release.digest(release.INCOMING / 'covers.tar')}}
            release.stage_covers(manifest)
            release.stage_covers(manifest)
            self.assertEqual(len(list((release.ROOT / 'covers').iterdir())), 250)
            self.assertEqual(release.cover_mount(), ['-v', str(release.ROOT / 'covers') + ':/app/covers:ro'])
            extra = b'an additional catalog cover'
            sha = hashlib.sha256(extra).hexdigest()
            member = tarfile.TarInfo(sha + '.png')
            member.size = len(extra)
            with tarfile.open(release.INCOMING / 'covers.tar', 'w') as bundle:
                bundle.addfile(member, io.BytesIO(extra))
            manifest['cover_files'] = {member.name: sha}
            manifest['sha256']['covers.tar'] = release.digest(release.INCOMING / 'covers.tar')
            release.stage_covers(manifest)
            release.stage_covers(manifest)
            self.assertEqual(len(list((release.ROOT / 'covers').iterdir())), 251)
            for name, old_sha in files.items():
                self.assertEqual(release.digest(release.ROOT / 'covers' / name), old_sha)
            files = manifest['cover_files']
            victim = release.ROOT / 'covers' / next(iter(files))
            victim.write_bytes(b'changed')
            with self.assertRaises(AssertionError):
                release.stage_covers(manifest)
            self.assertEqual(victim.read_bytes(), b'changed')
            invalid = dict(files)
            sha = invalid.pop(next(iter(invalid)))
            invalid['../outside.png'] = sha
            manifest['cover_files'] = invalid
            with self.assertRaises(AssertionError):
                release.stage_covers(manifest)


if __name__ == '__main__':
    unittest.main()
