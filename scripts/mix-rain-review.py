"""Render the authorised, fixed CC0 rain mixes locally, without touching the bot."""
import concurrent.futures
import copy
import hashlib
import importlib.util
import json
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('station_review', ROOT / 'scripts/prepare-station-review.py')
qa = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qa)
RECIPE_VERSION = 2

def trailing_end(quality):
    duration = quality['duration_seconds']
    starts = [float(x.split(': ')[1]) for x in quality['silence_events'] if x.startswith('silence_start:')]
    ends = [float(x.split(': ')[1]) for x in quality['silence_events'] if x.startswith('silence_end:')]
    return min(duration, starts[-1] + 0.25) if starts and ends and abs(ends[-1] - duration) < 0.1 else duration

def render(track, rain):
    original = track.get('original_music') or copy.deepcopy(track)
    if track['title'] == 'School day / Rain / Sun / Loop' and not track.get('original_music'):
        original['file'] = 'media/rain/school-day-clean-base.ogg'
        original['download_url'] = 'https://opengameart.org/sites/default/files/SchoolDay.ogg'
        original.pop('technical_review', None)
        original['source_variant'] = 'SchoolDay.ogg, the unlayered composition from the same CC0 source page'
    original = qa.prepare(original)
    q = original['technical_review']
    duration = trailing_end(q)
    music_gain = min(20, -18 - q['input_lufs'], -6 - q['input_true_peak_dbfs'])
    rain_quality = rain['technical_review']
    rain_gain = min(-30 - rain_quality['input_lufs'], -12 - rain_quality['input_true_peak_dbfs'])
    offset = int(hashlib.sha256(q['sha256'].encode()).hexdigest()[:8], 16) % 16
    assert duration + offset < rain_quality['duration_seconds'], 'Rain layer too short; do not loop or truncate without review'
    recipe = {'version': RECIPE_VERSION, 'music_sha256': q['sha256'], 'rain_sha256': rain['sha256'], 'music_gain_db': music_gain, 'rain_gain_db': rain_gain, 'rain_offset_seconds': offset, 'duration_seconds': duration, 'rain_fade_seconds': 2, 'encoding': 'Ogg Vorbis q6, 48000 Hz stereo', 'trimmed_music_tail_seconds': q['duration_seconds'] - duration}
    file = 'media/rain/' + Path(original['file']).stem + '-rain-mix.ogg'
    destination = (qa.REVIEW / file).resolve()
    assert destination.is_relative_to(qa.REVIEW.resolve())
    if not destination.exists() or track.get('mix_recipe') != recipe:
        temporary = destination.with_suffix('.rendering.ogg')
        fade_start = max(0, duration - 2)
        filters = f'[0:a]aresample=48000,asetpts=N/SR/TB,atrim=duration={duration},volume={music_gain}dB,afade=t=out:st={duration-0.05}:d=0.05[m];[1:a]aresample=48000,asetpts=N/SR/TB,atrim=start={offset}:end={offset+duration},asetpts=N/SR/TB,volume={rain_gain}dB,afade=t=in:d=2,afade=t=out:st={fade_start}:d=2[r];[m][r]amix=inputs=2:duration=first:dropout_transition=0:normalize=0,asetpts=N/SR/TB[out]'
        # Some source OGGs carry discontinuous timestamps; use decoded sample counts.
        qa.run('ffmpeg', '-hide_banner', '-v', 'error', '-xerror', '-nostdin', '-y', '-i', str(qa.REVIEW / original['file']), '-i', str(ROOT / rain['file']), '-filter_complex', filters, '-map', '[out]', '-ar', '48000', '-ac', '2', '-c:a', 'libvorbis', '-q:a', '6', '-map_metadata', '-1', '-metadata', 'title=' + track['title'] + ' (Rain mix)', '-metadata', 'artist=' + original['artist'] + ' + ' + rain['artist'], '-metadata', 'comment=Fixed CC0 mix. Music: ' + original['source_url'] + ' | Rain: ' + rain['source_url'], str(temporary))
        temporary.replace(destination)
        track.pop('technical_review', None)
    track.update(file=file, original_music=original, mix_recipe=recipe, rain_source=rain, artist=original['artist'] + ' + ' + rain['artist'], approved=False)
    track['human_listening_review'] = {'status': 'pending', 'reviewer': None, 'full_recording': False, 'no_voice': None, 'station_suitability': None}
    track['rain_review'] = {'status': 'fixed-mix-rendered-awaiting-listening', 'method': 'Two CC0 sources mixed offline; no runtime mixer', 'integrated_rain_rendered': True, 'integrated_rain_verified_by_listening': False}
    track = qa.prepare(track)
    assert track['technical_review']['input_true_peak_dbfs'] <= -2, 'Rendered mix needs peak correction'
    assert not track['technical_review']['silence_events'], 'Rendered mix contains a long silent section'
    print('Rain mix ready:', track['title'], flush=True)
    return track

def main():
    manifest = json.loads(qa.MANIFEST.read_text(encoding='utf8'))
    rain = json.loads((ROOT / 'licenses/expansion-review/rain-layer-20261031.json').read_text())
    rain['file'] = rain['file'].replace('\\', '/')
    path = (ROOT / rain['file']).resolve()
    assert path.is_relative_to((ROOT / 'output/station-research').resolve())
    if not path.exists():
        url = urllib.parse.urlsplit(rain['download_url'])
        assert url.scheme == 'https' and url.hostname == 'opengameart.org'
        path.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(rain['download_url'], timeout=60) as response, path.with_suffix('.part').open('wb') as out:
            while chunk := response.read(1024 * 1024):
                out.write(chunk)
                assert out.tell() < 100 * 1024 * 1024
        path.with_suffix('.part').replace(path)
    assert hashlib.sha256(path.read_bytes()).hexdigest() == rain['sha256']
    measured = Path(str(path) + '.qa.json')
    if not measured.exists():
        # Reuse the existing full decoder and loudness analyser for the layer itself.
        layer_file = qa.REVIEW / 'media/rain-layer.ogg'
        layer_file.write_bytes(path.read_bytes())
        result = qa.prepare({'file': 'media/rain-layer.ogg', 'title': rain['title'], 'station': 'rain-layer'})
        measured.write_text(json.dumps(result['technical_review'], indent=2))
    rain['technical_review'] = json.loads(measured.read_text())
    assert rain['technical_review']['sha256'] == rain['sha256']
    (ROOT / 'licenses/expansion-review/rain-layer-20261031.json').write_text(json.dumps(rain, indent=2) + '\n')
    selected = [t for t in manifest['tracks'] if t['station'] == 'rain']
    assert len(selected) == 25
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as workers:
        mixed = list(workers.map(lambda t: render(t, rain), selected))
    assert len({t['original_music']['technical_review']['pcm_sha256'] for t in mixed}) == 25
    by_title = {t['title']: t for t in mixed}
    manifest['tracks'] = [by_title[t['title']] if t['station'] == 'rain' else t for t in manifest['tracks']]
    manifest['note'] = 'CC0 declarations checked on original source pages. User authorised fixed offline rain mixes on 2026-10-31. Full human listening and station suitability remain pending; no live catalog change.'
    manifest['rain_mix_authorisation'] = {'date': '2026-10-31', 'scope': '25 fixed offline instrumental-plus-rain mixes with both authors credited'}
    qa.MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf8')
    qa.main()

if __name__ == '__main__':
    assert trailing_end({'duration_seconds': 100, 'silence_events': []}) == 100
    assert trailing_end({'duration_seconds': 100, 'silence_events': ['silence_start: 94', 'silence_end: 100']}) == 94.25
    assert trailing_end({'duration_seconds': 100, 'silence_events': ['silence_start: 40', 'silence_end: 45']}) == 100
    main()
