"""Collect CC0/CC BY source evidence; never edit catalog.json or prior curation."""
import concurrent.futures
import hashlib
import html
import json
import re
import urllib.parse
import urllib.request
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / 'output/six-stations-research'
CACHE.mkdir(parents=True, exist_ok=True)
MANIFEST = ROOT / 'licenses/expansion-review/curation-ccby-20261102.json'
SEEDS = {
    'synthwave': ['retro-wave-collection', 'synthwavetype', 'space-synth-wave', 'dj-synth-wave-funk', 'vintage-menu', 'slipstream'],
    'acoustic': ['sunset-plains'],
    'chiptune': ['chill-chiptunes-collection', 'happy-chiptunes-collection', '10-more-chiptune-tracks-another-bag-of-chips', 'mecha-collection'],
    'chill-house': ['synthwave-house-loop'],
    'drum-and-bass': ['drum-and-bass', 'final-hour', 'black-diamond', 'royalty-free-game-music-loops'],
    'bossa-nova': ['bossa-town', 'bossa-nova', 'two-left-socks', 'shop-theme'],
}
TAGS = {
    'synthwave': ['synthwave', 'synth wave', 'retro wave'],
    'acoustic': ['acoustic guitar', 'acoustic'],
    'chiptune': ['chiptune'],
    'chill-house': ['chill house', 'deep house', 'house'],
    'drum-and-bass': ['drum and bass', 'dnb', 'liquid'],
    'bossa-nova': ['bossa nova', 'bossa'],
}


def fetch(url):
    assert urllib.parse.urlsplit(url).hostname == 'opengameart.org'
    file = CACHE / (hashlib.sha256(url.encode()).hexdigest() + '.html')
    if not file.exists():
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=35) as response:
            data = response.read(4 * 1024 * 1024 + 1)
            assert len(data) <= 4 * 1024 * 1024
        temp = file.with_suffix('.part')
        temp.write_bytes(data)
        temp.replace(file)
    return file.read_text(encoding='utf8')


def text(markup):
    return ' '.join(html.unescape(re.sub('<[^>]+>', ' ', markup)).split())


def field(page, name):
    match = re.search(r'<div class="field field-name-' + name + r'\b.*?(?=<div class="field field-name-|$)', page, re.S)
    return match[0] if match else ''


def links(markup):
    return [html.unescape(u) for u in re.findall(r'href=["\x27](.*?)["\x27]', markup, re.S)]


def source(item):
    station, url = item
    page = fetch(url)
    license_field = field(page, 'field-art-licenses')
    licenses = [u for u in links(license_field) if 'creativecommons.org/' in u]
    allowed = {'/publicdomain/zero/1.0/': 'CC0-1.0', '/licenses/by/4.0/': 'CC-BY-4.0', '/licenses/by/3.0/': 'CC-BY-3.0'}
    declared = {allowed.get(urllib.parse.urlsplit(u).path.rstrip('/') + '/') for u in licenses if urllib.parse.urlsplit(u).hostname in {'creativecommons.org', 'www.creativecommons.org'}}
    license_id = next((value for value in allowed.values() if value in declared), None)
    if not license_id:
        return None
    if 'Music' not in text(field(page, 'field-art-type')):
        return None
    downloads = [u for u in links(field(page, 'field-art-files')) if '/sites/default/files/' in u and re.search(r'\.(?:mp3|ogg|wav|flac|zip)(?:\?|$)', u, re.I)]
    title = text(field(page, 'title'))
    author = text(field(page, 'author-submitter')).removeprefix('Author:').strip()
    assert title and author, 'Missing source title or author'
    return {
        'station': station, 'source_url': url, 'source_title': title,
        'artist': author,
        'license': license_id, 'license_urls': licenses,
        'credit_notice': text(field(page, 'field-copyright-notice')).removeprefix('Copyright/Attribution Notice:').strip(),
        'credit_notice_urls': links(field(page, 'field-copyright-notice')),
        'license_verified_on': str(date.today()),
        'source_page_sha256': hashlib.sha256(page.encode()).hexdigest(),
        'source_tags': text(field(page, 'field-art-tags')).removeprefix('Tags:').strip(),
        'source_description_excerpt': ' '.join(text(field(page, 'body')).split()[:12]),
        'source_description_sha256': hashlib.sha256(text(field(page, 'body')).encode()).hexdigest(),
        'download_urls': sorted(set(downloads)), 'approved': False,
    }


def main():
    jobs = {(s, 'https://opengameart.org/content/' + slug) for s, slugs in SEEDS.items() for slug in slugs}
    errors = []
    for station, tags in TAGS.items():
        queries = [(tag, page) for tag in tags for page in range(3)]
        for tag, index in queries:
            url = 'https://opengameart.org/art-search-advanced?' + urllib.parse.urlencode({'field_art_tags_tid': tag, 'field_art_type_tid[]': 12, **({'page': index} if index else {})})
            try:
                page = fetch(url)
                # Only result links from the main content, never sidebar recommendations.
                content = page.split('id="content"', 1)[-1]
                found = re.findall(r'<span class="art-preview-title">\s*<a href="(/content/[^"]+)"', content)
                for href in found[:30]:
                    jobs.add((station, 'https://opengameart.org' + href))
                print(station, tag, len(found), 'source pages', flush=True)
            except Exception as error:
                errors.append({'url': url, 'error': str(error)})
    sources = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        futures = {pool.submit(source, item): item for item in sorted(jobs)}
        for future in concurrent.futures.as_completed(futures):
            try:
                record = future.result()
                if record:
                    sources.append(record)
            except Exception as error:
                errors.append({'url': futures[future][1], 'error': str(error)})
    sources.sort(key=lambda s: (s['station'], s['source_title']))
    result = {'reviewed_on': str(date.today()), 'scope': 'Candidate sources only. CC0 and CC BY 3.0/4.0 licence declarations are not a full rights-chain investigation. Attribution, genre, voice content and subjective quality require review.', 'minimum_tracks_per_station': 25, 'sources': sources, 'tracks': [], 'research_errors': errors, 'comparison_manifests': ['licenses/expansion-review/curation-20261102.json']}
    if MANIFEST.exists():
        result['tracks'] = json.loads(MANIFEST.read_text(encoding='utf8')).get('tracks', [])
    MANIFEST.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n', encoding='utf8')
    print(json.dumps({s: sum(x['station'] == s for x in sources) for s in SEEDS}), flush=True)
    print('Errors:', len(errors), flush=True)


if __name__ == '__main__':
    main()
