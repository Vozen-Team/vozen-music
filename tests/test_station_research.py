import importlib.util
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


research = load('research-six-stations')
candidates = load('prepare-six-station-candidates')


def page(license_url):
    return f'''<div class="field field-name-title"><h2>Quiet &amp; Warm</h2></div>
      <div class="field field-name-author-submitter">Author: <a href="/users/composer">Composer</a></div>
      <div class="field field-name-field-art-type">Music</div>
      <div class="field field-name-field-art-licenses"><a href="{license_url}">Licence</a></div>
      <div class="field field-name-body">A collection mentioning CC0 in its prose.</div>
      <div class="field field-name-field-art-files"><a href="https://opengameart.org/sites/default/files/song.ogg">song</a></div>
      <div class="field field-name-next">End</div><a class="username">Commenter</a>'''


class ResearchTests(unittest.TestCase):
    def test_uses_original_author_and_file_field(self):
        with patch.object(research, 'fetch', return_value=page('https://creativecommons.org/publicdomain/zero/1.0/')):
            source = research.source(('acoustic', 'https://opengameart.org/content/quiet'))
        self.assertEqual(source['artist'], 'Composer')
        self.assertEqual(source['source_title'], 'Quiet & Warm')
        self.assertFalse(source['approved'])
        self.assertEqual(source['download_urls'], ['https://opengameart.org/sites/default/files/song.ogg'])

    def test_prose_mentioning_cc0_does_not_override_actual_license(self):
        with patch.object(research, 'fetch', return_value=page('https://creativecommons.org/licenses/by-nc/4.0/')):
            self.assertIsNone(research.source(('acoustic', 'https://opengameart.org/content/quiet')))

    def test_only_the_approved_by_versions_and_exact_license_host_are_allowed(self):
        for version in ['3.0', '4.0']:
            with patch.object(research, 'fetch', return_value=page(f'https://creativecommons.org/licenses/by/{version}/')):
                self.assertEqual(research.source(('acoustic', 'https://opengameart.org/content/quiet'))['license'], 'CC-BY-' + version)
        for uri in ['https://creativecommons.org/licenses/by-sa/4.0/', 'https://creativecommons.org/licenses/by-nd/4.0/', 'https://evil.example/creativecommons.org/licenses/by/4.0/']:
            with patch.object(research, 'fetch', return_value=page(uri)):
                self.assertIsNone(research.source(('acoustic', 'https://opengameart.org/content/quiet')))

    def test_format_copies_are_grouped_and_download_host_is_restricted(self):
        self.assertEqual(candidates.title_key('Lone%20Scavenger_4.mp3'), candidates.title_key('Lone Scavenger_0.wav'))
        self.assertNotEqual(candidates.title_key('DnB_1.wav', uploaded=False), candidates.title_key('DnB_2.wav', uploaded=False))
        with self.assertRaises(AssertionError):
            candidates.download('https://example.com/song.ogg')


if __name__ == '__main__':
    unittest.main()
