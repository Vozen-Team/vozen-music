"""Prepare unapproved, local listening copies; this never edits the active catalog."""
import concurrent.futures
import hashlib
import json
import math
import re
import subprocess
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / 'licenses/expansion-review/curation-20261031.json'
REVIEW = ROOT / 'output/station-review'
REVIEW.mkdir(parents=True, exist_ok=True)

def run(*args):
    return subprocess.run(args, capture_output=True, text=True, encoding='utf8', check=True, creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))

def prepare(track):
    destination = (REVIEW / track['file']).resolve()
    assert destination.is_relative_to(REVIEW.resolve())
    destination.parent.mkdir(parents=True, exist_ok=True)
    if not destination.exists():
        temporary = destination.with_suffix('.part')
        if track.get('archive'):
            archive = (ROOT / track['archive']).resolve()
            assert archive.is_relative_to((ROOT / 'output/station-research').resolve())
            if not archive.exists():
                archive.parent.mkdir(parents=True, exist_ok=True)
                url = urllib.parse.urlsplit(track['download_url'])
                assert url.scheme == 'https' and url.hostname == 'opengameart.org'
                with urllib.request.urlopen(track['download_url'], timeout=60) as response, archive.with_suffix('.part').open('wb') as out:
                    while chunk := response.read(1024 * 1024):
                        out.write(chunk)
                        assert out.tell() < 250 * 1024 * 1024
                archive.with_suffix('.part').replace(archive)
            archive_digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            assert not track.get('archive_sha256') or archive_digest == track['archive_sha256'], 'Source archive changed; re-review required'
            with zipfile.ZipFile(archive) as pack:
                info = pack.getinfo(track['archive_member'])
                assert 0 < info.file_size < 150 * 1024 * 1024
                temporary.write_bytes(pack.read(info))
            track['archive_sha256'] = archive_digest
        else:
            url = urllib.parse.urlsplit(track['download_url'])
            assert url.scheme == 'https' and url.hostname == 'opengameart.org'
            with urllib.request.urlopen(track['download_url'], timeout=60) as response, temporary.open('wb') as out:
                while chunk := response.read(1024 * 1024):
                    out.write(chunk)
                    assert out.tell() < 150 * 1024 * 1024
        temporary.replace(destination)
    digest = hashlib.sha256(destination.read_bytes()).hexdigest()
    expected = track.get('technical_review', {}).get('sha256')
    assert not expected or digest == expected, 'Source audio changed; re-review required'
    cached = destination.with_suffix(destination.suffix + '.qa.json')
    if cached.exists():
        quality = json.loads(cached.read_text())
        if quality['sha256'] == digest:
            track['technical_review'] = quality
            return track
    info = json.loads(run('ffprobe', '-v', 'error', '-show_format', '-show_streams', '-of', 'json', str(destination)).stdout)
    audio = next(s for s in info['streams'] if s['codec_type'] == 'audio')
    duration = float(info['format']['duration'])
    assert duration > 10, f"{track['title']}: shorter than 10 seconds"
    assert int(audio['sample_rate']) >= 44100, f"{track['title']}: source below 44100 Hz"
    pcm_hash = run('ffmpeg', '-v', 'error', '-xerror', '-nostdin', '-i', str(destination), '-map', '0:a:0', '-ac', '2', '-ar', '48000', '-f', 'hash', '-hash', 'sha256', '-').stdout.strip().split('=')[1]
    measurement = run('ffmpeg', '-hide_banner', '-nostdin', '-i', str(destination), '-vn', '-af', 'silencedetect=noise=-50dB:d=3,loudnorm=I=-18:TP=-2:LRA=11:print_format=json', '-f', 'null', '-').stderr
    stats = json.loads(measurement[measurement.rfind('{'):measurement.rfind('}') + 1])
    loudness, peak = float(stats['input_i']), float(stats['input_tp'])
    assert math.isfinite(loudness) and math.isfinite(peak)
    gain = min(20, -18 - loudness, -2 - peak)
    quality = {'sha256': digest, 'pcm_sha256': pcm_hash, 'bytes': destination.stat().st_size, 'duration_seconds': duration, 'sample_rate': int(audio['sample_rate']), 'channels': audio['channels'], 'codec': audio['codec_name'], 'bit_rate': int(info['format'].get('bit_rate', 0)), 'decoded_in_full': True, 'input_lufs': loudness, 'input_true_peak_dbfs': peak, 'loudness_range_lu': float(stats['input_lra']), 'gain_db': round(gain, 6), 'projected_lufs': loudness + gain, 'projected_true_peak_dbfs': peak + gain, 'silence_events': re.findall(r'silence_(?:start|end|duration): [0-9.]+', measurement), 'voice_content_checked': False, 'subjective_quality_checked': False}
    cached.write_text(json.dumps(quality, indent=2) + '\n')
    track['technical_review'] = quality
    print(track['station'], track['title'], f'{duration:.0f}s', 'decoded', flush=True)
    return track

def main():
    manifest = json.loads(MANIFEST.read_text(encoding='utf8'))
    errors, completed = [], []
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as workers:
        futures = {workers.submit(prepare, track): track for track in manifest['tracks']}
        for future in concurrent.futures.as_completed(futures):
            try:
                completed.append(future.result())
            except Exception as error:
                errors.append({'file': futures[future]['file'], 'error': str(error)})
    completed.sort(key=lambda t: (t['station'], t['title']))
    old = json.loads((ROOT / 'licenses/catalog-sources.json').read_text(encoding='utf8'))['tracks']
    old_pcm = {x['pcm_sha256'] for x in old}
    seen = set()
    for track in completed:
        pcm = track['technical_review']['pcm_sha256']
        if pcm in old_pcm or pcm in seen:
            errors.append({'file': track['file'], 'error': 'Duplicate decoded recording; replace before approval'})
        seen.add(pcm)
    by_file = {t['file']: t for t in completed}
    manifest['tracks'] = [by_file.get(t['file'], t) for t in manifest['tracks']]
    manifest['technical_errors'] = errors
    MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf8')
    (REVIEW / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf8')
    print(f'Completed {len(completed)}/75; {len(errors)} technical errors. Human listening and Rain suitability remain unapproved.', flush=True)
    for error in errors:
        print(error, flush=True)
    assert not errors
    assert len(completed) == 75 and len(seen) == 75
    assert all(not t['approved'] for t in completed), 'A technical test cannot approve voice content'

if __name__ == '__main__':
    main()
