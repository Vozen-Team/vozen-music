"""Prepare source-declared CC0 candidates for full listening; no automatic approval."""
import hashlib
import json
import re
import subprocess
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / 'licenses/expansion-review/curation-20261102.json'
REVIEW = ROOT / 'output/six-stations-review'
DOWNLOADS = ROOT / 'output/station-research/downloads'
DOWNLOADS.mkdir(parents=True, exist_ok=True)
AUDIO = {'.ogg', '.mp3', '.wav', '.flac'}


def download(url):
    parsed = urllib.parse.urlsplit(url)
    assert parsed.scheme == 'https' and parsed.hostname == 'opengameart.org'
    dest = DOWNLOADS / (hashlib.sha256(url.encode()).hexdigest()[:16] + Path(parsed.path).suffix)
    if not dest.exists():
        temp = dest.with_suffix('.part')
        request = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(request, timeout=60) as response, temp.open('wb') as out:
            while chunk := response.read(1024 * 1024):
                out.write(chunk)
                assert out.tell() <= 250 * 1024 * 1024, 'Oversized download'
        temp.replace(dest)
    return dest


def title_key(name, uploaded=True):
    stem = Path(urllib.parse.unquote(name)).stem.lower()
    return re.sub(r'[^a-z0-9]', '', re.sub(r'_\d+$', '', stem) if uploaded else stem)


def main():
    manifest = json.loads(MANIFEST.read_text(encoding='utf8'))
    old = json.loads((ROOT / 'licenses/catalog-sources.json').read_text(encoding='utf8'))['tracks']
    old_hashes = {r.get('original_sha256', r['sha256']) for r in old} | {r['sha256'] for r in old}
    used, tracks, exclusions, failures = set(), [], [], []
    # Prioritize sources dedicated to the genre, then individual uploads.
    sources = sorted(manifest['sources'], key=lambda s: (s['station'], 'collection' not in s['source_url'], s['source_title']))
    for source in sources:
        station = source['station']
        if source['source_url'].endswith(('bossa-shop-theme-in-low-fi-and-hd', 'unfinishedscrapseven-less-good-tracks')):
            exclusions.append({'source_url': source['source_url'], 'reason': 'Sample-library rights or explicitly unfinished quality require a separate review'})
            continue
        if sum(t['station'] == station for t in tracks) >= 30:
            continue
        choices = {}
        for url in source['download_urls']:
            if 'prior-versions' in url:
                continue
            suffix = Path(urllib.parse.urlsplit(url).path).suffix.lower()
            if suffix not in AUDIO | {'.zip'}:
                continue
            key = title_key(url, uploaded=suffix != '.zip')
            if key not in choices or suffix in {'.mp3', '.ogg'}:
                choices[key] = url
        multi_track = re.search(r'collection|\b\d+ (?:more )?(?:chiptune )?tracks\b', source['source_title'], re.I)
        if not multi_track and not source['source_url'].endswith(('post-apocalyptic-morning-afternoon-and-night', 'royalty-free-game-music-loops')):
            preferred = sorted(choices.values(), key=lambda u: (Path(u).suffix not in {'.ogg', '.mp3'}, u))
            choices = {title_key(preferred[0]): preferred[0]} if preferred else {}
        for url in choices.values():
            try:
                downloaded = download(url)
                archive = None
                if downloaded.suffix == '.zip':
                    archive = zipfile.ZipFile(downloaded)
                    audio = [x for x in archive.infolist() if Path(x.filename).suffix.lower() in AUDIO and not x.filename.startswith('__MACOSX/')]
                    assert sum(x.file_size for x in audio) <= 500 * 1024 * 1024, 'Oversized source archive'
                    selected = {}
                    for info in audio:
                        key = title_key(info.filename, uploaded=False)
                        if key not in selected or Path(info.filename).suffix.lower() in {'.mp3', '.ogg'}:
                            selected[key] = info
                    entries = [(x.filename, archive.read(x)) for x in selected.values() if 0 < x.file_size <= 150 * 1024 * 1024]
                else:
                    entries = [(urllib.parse.unquote(urllib.parse.urlsplit(url).path.rsplit('/', 1)[-1]), downloaded.read_bytes())]
                for name, data in entries:
                    sha = hashlib.sha256(data).hexdigest()
                    reason = 'Already in the active catalog' if sha in old_hashes else 'Duplicate source audio' if sha in used else None
                    if re.search(r'\b(vocals?|singing|intro|jingle|sting|sfx|preview|radio (?:version|edit))\b', name, re.I):
                        reason = 'Vocal, preview or short-form variant indicated by filename'
                    if source['source_url'].endswith('royalty-free-game-music-loops') and station == 'drum-and-bass' and 'dnb' not in name.lower():
                        reason = 'Source archive labels this recording as another genre'
                    if source['source_url'].endswith('short-looping-songs') and station == 'acoustic' and 'acoustic' not in name.lower():
                        reason = 'Source archive labels this recording as another genre'
                    if reason:
                        exclusions.append({'station': station, 'source_url': source['source_url'], 'name': name, 'reason': reason})
                        continue
                    if sum(t['station'] == station for t in tracks) >= 30:
                        break
                    used.add(sha)
                    relative = f'media/{station}/{sha[:20]}{Path(name).suffix.lower()}'
                    destination = REVIEW / relative
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    if not destination.exists():
                        destination.write_bytes(data)
                    info = subprocess.run(['ffprobe', '-v', 'error', '-show_format', '-of', 'json', str(destination)], capture_output=True, text=True, encoding='utf8', check=True, creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
                    tags = {k.lower(): v for k, v in json.loads(info.stdout)['format'].get('tags', {}).items()}
                    title = tags.get('title') or (re.sub(r'_\d+$', '', Path(name).stem).replace('_', ' ') if archive or multi_track else source['source_title'])
                    track = {k: source[k] for k in ['station', 'source_url', 'license', 'license_urls', 'license_verified_on', 'source_page_sha256', 'source_title', 'source_tags']}
                    track.update(title=title, artist=tags.get('artist') or source['artist'], source_author=source['artist'], download_url=url, file=relative, approved=False, human_listening_review={'status': 'pending', 'reviewer': None, 'full_recording': False, 'no_voice': None, 'station_suitability': None})
                    if archive:
                        track.update(archive=str(downloaded.relative_to(ROOT)).replace('\\', '/'), archive_member=name, archive_sha256=hashlib.sha256(downloaded.read_bytes()).hexdigest())
                    tracks.append(track)
                if archive:
                    archive.close()
                print(station, source['source_title'], sum(t['station'] == station for t in tracks), 'candidates', flush=True)
            except Exception as error:
                failures.append({'url': url, 'error': str(error)})
            finally:
                manifest.update(tracks=tracks, candidate_exclusions=exclusions, preparation_errors=failures)
                MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf8')
    print(json.dumps({s: sum(t['station'] == s for t in tracks) for s in sorted({x['station'] for x in sources})}), flush=True)
    print('All candidates remain unapproved. Failures:', len(failures), flush=True)


if __name__ == '__main__':
    main()
