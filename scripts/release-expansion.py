"""Run as root on the verified VPS: stage first, then apply. Never opens a gateway in staging."""
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import time
import tarfile
from tempfile import TemporaryDirectory
import urllib.request

ROOT = Path('/srv/vozen-secure/music')
INCOMING = Path(os.environ.get('MUSIC_RELEASE_INCOMING', '/home/ubuntu/vozen-music-expansion-20261031'))
CONTAINER = 'vozen-music-music-1'

def cover_mount():
    return ['-v', str(ROOT / 'covers') + ':/app/covers:ro'] if (ROOT / 'covers').exists() else []

def stage_covers(manifest):
    expected = manifest.get('cover_files')
    if not expected:
        return
    assert all(len(sha) == 64 and set(sha) <= set('0123456789abcdef') and name == sha + '.png' for name, sha in expected.items())
    assert digest(INCOMING / 'covers.tar') == manifest['sha256']['covers.tar']
    target = ROOT / 'covers'
    assert not target.is_symlink()
    with TemporaryDirectory(dir=ROOT, prefix='covers-stage-') as folder:
        pending = Path(folder)
        with tarfile.open(INCOMING / 'covers.tar') as bundle:
            members = bundle.getmembers()
            assert len(members) == len(expected) and {m.name for m in members} == set(expected)
            for member in members:
                assert member.isfile() and 0 < member.size <= 8 * 1024 * 1024
                data = bundle.extractfile(member).read()
                assert hashlib.sha256(data).hexdigest() == expected[member.name]
                path = pending / member.name
                path.write_bytes(data)
                path.chmod(0o644)
        target.mkdir(mode=0o755, exist_ok=True)
        for name, sha in expected.items():
            path = target / name
            assert not path.is_symlink()
            if path.exists():
                assert path.is_file() and digest(path) == sha
        for name in expected:
            path = target / name
            try:
                os.link(pending / name, path)
            except FileExistsError:
                assert not path.is_symlink() and digest(path) == expected[name]
    assert all(digest(target / name) == sha for name, sha in expected.items())

def run(*args):
    return subprocess.check_output(args, text=True).strip()

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def atomic(path, data):
    pending = path.with_name(path.name + '.expansion-pending')
    pending.write_bytes(data)
    pending.chmod(0o600)
    pending.replace(path)

def normalized(policies):
    return {str(g): {**p, 'moderator_roles': sorted(map(int, p['moderator_roles']))} for g, p in policies.items()}

def verify_migration(folder, original):
    with closing(sqlite3.connect(folder / 'skip-settings.sqlite3')) as db:
        assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        assert db.execute('PRAGMA foreign_key_check').fetchall() == []
        rows = {g: json.loads(body)['skip'] for g, body in db.execute('SELECT guild,body FROM settings')}
        assert normalized(rows) == normalized(original)

def migrate_copy(folder, tag, data, database=None):
    folder.mkdir(mode=0o700)
    os.chown(folder, 10001, 10001)
    legacy = folder / 'skip-settings.json'
    legacy.write_bytes(data)
    legacy.chmod(0o600)
    os.chown(legacy, 10001, 10001)
    expected = json.loads(data)
    if database is not None and database.exists():
        target = folder / 'skip-settings.sqlite3'
        with closing(sqlite3.connect(database)) as source, closing(sqlite3.connect(target)) as snapshot:
            source.backup(snapshot)
            expected = {g: json.loads(body)['skip'] for g, body in snapshot.execute('SELECT guild,body FROM settings')}
        os.chown(target, 10001, 10001)
    run('docker', 'run', '--rm', '--network=none', '--read-only', '--cap-drop=ALL', '-v', f'{folder}:/app/data', *cover_mount(), tag, '--check-data')
    verify_migration(folder, expected)
    assert legacy.read_bytes() == data

def set_image(tag):
    path = ROOT / '.env'
    lines = path.read_text().splitlines()
    assert sum(line.startswith('MUSIC_IMAGE=') for line in lines) == 1
    atomic(path, ('\n'.join('MUSIC_IMAGE=' + tag if line.startswith('MUSIC_IMAGE=') else line for line in lines) + '\n').encode())

def healthy():
    for _ in range(35):
        try:
            with urllib.request.urlopen('http://127.0.0.1:8790/music/health', timeout=2) as response:
                if response.status == 200 and run('systemctl', 'is-active', 'vozen-music') == 'active':
                    return
        except Exception:
            pass
        time.sleep(1)
    raise RuntimeError('Music did not become healthy.')

def siblings():
    names = run('docker', 'ps', '--format', '{{.Names}}').splitlines()
    names = [n for n in names if 'vozen' in n.lower() and n != CONTAINER]
    return {n: run('docker', 'inspect', n, '--format', '{{.Image}} {{.State.StartedAt}}') for n in names}

def discord(env, suffix, method='GET', body=None):
    token = env['DISCORD_TOKEN'].removeprefix('Bot ')
    request = urllib.request.Request('https://discord.com/api/v10/' + suffix, data=None if body is None else json.dumps(body).encode(), method=method,
                                    headers={'Authorization': 'Bot ' + token, 'Content-Type': 'application/json', 'User-Agent': 'VozenMusicRelease/1.0'})
    with urllib.request.urlopen(request, timeout=15) as response:
        return json.load(response)

def main():
    assert sys.argv[1:] in [['stage'], ['apply']]
    assert os.geteuid() == 0 and run('hostname') == 'vps-4a10dd35'
    subprocess.run(['mountpoint', '-q', '/srv/vozen-secure'], check=True)
    manifest = json.loads((INCOMING / 'manifest.json').read_text())
    for name in ['vozen-music', 'catalog.json']:
        assert digest(INCOMING / name) == manifest['sha256'][name]
    previous = manifest['previous_image']
    assert run('docker', 'inspect', CONTAINER, '--format', '{{.Config.Image}}') == previous
    assert shutil.disk_usage(ROOT).free > 3 * (INCOMING / 'vozen-music').stat().st_size + 100 * 1024 * 1024
    stage_root = ROOT
    if manifest.get('bundle_files'):
        assert digest(INCOMING / 'stations.tar.gz') == manifest['sha256']['stations.tar.gz']
        stage_root = Path('/var/tmp/vozen-music-release-stage')
        stage_root.mkdir(mode=0o700, exist_ok=True)
        assert stage_root.is_dir() and not stage_root.is_symlink()
        assert stage_root.stat().st_uid == 0 and stage_root.stat().st_mode & 0o777 == 0o700
        if sys.argv[1] == 'stage':
            with tarfile.open(INCOMING / 'stations.tar.gz', 'r:gz') as bundle:
                unpacked_bytes = sum(member.size for member in bundle.getmembers())
            assert shutil.disk_usage(stage_root).free > 2 * unpacked_bytes + 2 * (INCOMING / 'vozen-music').stat().st_size + 100 * 1024 * 1024
            docker_root = run('docker', 'info', '--format', '{{.DockerRootDir}}')
            assert shutil.disk_usage(docker_root).free > 2 * unpacked_bytes + 100 * 1024 * 1024
    stage = stage_root / ('expansion-stage-' + manifest['source_revision'][:8])
    tag = 'vozen-music:expansion-' + manifest['source_revision'][:8]
    legacy_path = ROOT / 'data/skip-settings.json'
    if sys.argv[1] == 'stage':
        stage_covers(manifest)
        stage.mkdir(mode=0o700)
        for name in ['vozen-music', 'catalog.json']:
            shutil.copy2(INCOMING / name, stage / name)
        if manifest.get('bundle_files'):
            with tarfile.open(INCOMING / 'stations.tar.gz', 'r:gz') as bundle:
                assert {m.name for m in bundle.getmembers()} == set(manifest['bundle_files'])
                for member in bundle.getmembers():
                    target = (stage / member.name).resolve()
                    assert member.isfile() and target.is_relative_to(stage.resolve())
                    assert member.name.startswith(('media/expansion/', 'licenses/'))
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(bundle.extractfile(member).read())
                    target.chmod(0o644)
                    assert digest(target) == manifest['bundle_files'][member.name]
        (stage / 'vozen-music').chmod(0o755)
        (stage / 'Dockerfile').write_text(f'FROM {previous}\nCOPY vozen-music /usr/local/bin/vozen-music\nCOPY catalog.json /app/catalog.json\nLABEL org.opencontainers.image.revision="{manifest["source_revision"]}"\n')
        font_license = 'OFL-Outfit-player.txt'
        if font_license in manifest['sha256']:
            assert digest(INCOMING / font_license) == manifest['sha256'][font_license]
            shutil.copy2(INCOMING / font_license, stage / font_license)
            with (stage / 'Dockerfile').open('a') as dockerfile:
                dockerfile.write(f'COPY {font_license} /app/licenses/{font_license}\n')
        if manifest.get('bundle_files'):
            with (stage / 'Dockerfile').open('a') as dockerfile:
                dockerfile.write('COPY media/ /app/media/\nCOPY licenses/ /app/licenses/\n')
        subprocess.run(['docker', 'build', '--network=none', '-t', tag, str(stage)], check=True)
        run('docker', 'run', '--rm', '--network=none', '--read-only', '--cap-drop=ALL', *cover_mount(), tag, '--check')
        migrate_copy(stage / 'migration-test', tag, legacy_path.read_bytes() if legacy_path.exists() else b'{}', ROOT / 'data/skip-settings.sqlite3')
        receipt = {'sourceRevision': manifest['source_revision'], 'image': tag, 'imageId': run('docker', 'image', 'inspect', tag, '--format', '{{.Id}}'), 'migrationCopyPassed': True}
        (stage / 'staged.json').write_text(json.dumps(receipt, indent=2))
        print(json.dumps(receipt))
        return
    staged = json.loads((stage / 'staged.json').read_text())
    assert staged['migrationCopyPassed'] and staged['imageId'] == run('docker', 'image', 'inspect', tag, '--format', '{{.Id}}')
    backup = ROOT / ('expansion-before-' + manifest['source_revision'][:8])
    backup.mkdir(mode=0o700)
    before_siblings = siblings()
    env_bytes = (ROOT / '.env').read_bytes()
    compose_path = ROOT / 'compose.production.yaml'
    compose_bytes = compose_path.read_bytes()
    if manifest.get('cover_files') and 'compose.production.yaml' in manifest['sha256']:
        assert digest(INCOMING / 'compose.production.yaml') == manifest['sha256']['compose.production.yaml']
        candidate_compose = (INCOMING / 'compose.production.yaml').read_bytes()
        addition = b'      - type: bind\n        source: ./covers\n        target: /app/covers\n        read_only: true\n        bind:\n          create_host_path: false\n'
        assert candidate_compose.replace(addition, b'', 1) == compose_bytes
    else:
        candidate_compose = compose_bytes
    (backup / 'compose.production.yaml').write_bytes(compose_bytes)
    (backup / 'music.env').write_bytes(env_bytes)
    (backup / 'music.env').chmod(0o600)
    subprocess.run(['systemctl', 'stop', 'vozen-music'], check=True)
    try:
        shutil.copytree(ROOT / 'data', backup / 'data')
        (backup / 'data').chmod(0o700)
        original = legacy_path.read_bytes() if legacy_path.exists() else b'{}'
        migrate_copy(stage / 'quiescent-migration-test', tag, original, ROOT / 'data/skip-settings.sqlite3')
        assert (ROOT / '.env').read_bytes() == env_bytes
        set_image(tag)
        if candidate_compose != compose_bytes:
            atomic(compose_path, candidate_compose)
        subprocess.run(['systemctl', 'start', 'vozen-music'], check=True)
        healthy()
        assert run('docker', 'inspect', CONTAINER, '--format', '{{.Config.Image}}') == tag
        mounts = json.loads(run('docker', 'inspect', CONTAINER, '--format', '{{json .Mounts}}'))
        binds = {m['Destination']: (m['Source'], m['RW']) for m in mounts if m['Type'] == 'bind'}
        expected_binds = {'/app/data': (str(ROOT / 'data'), True)}
        if cover_mount():
            expected_binds['/app/covers'] = (str(ROOT / 'covers'), False)
        assert binds == expected_binds
        if manifest.get('cover_files'):
            assert all(digest(ROOT / 'covers' / name) == sha for name, sha in manifest['cover_files'].items())
        assert not legacy_path.exists() or legacy_path.read_bytes() == original
        expected = json.loads(original)
        if (backup / 'data/skip-settings.sqlite3').exists():
            with closing(sqlite3.connect(backup / 'data/skip-settings.sqlite3')) as before:
                expected = {g: json.loads(body)['skip'] for g, body in before.execute('SELECT guild,body FROM settings')}
        verify_migration(ROOT / 'data', expected)
        env = {line.split('=', 1)[0]: line.split('=', 1)[1].strip().strip('"').strip("'") for line in (ROOT / '.env').read_text().splitlines() if '=' in line and not line.startswith('#')}
        app = env['DISCORD_APPLICATION_ID']
        commands = discord(env, f'applications/{app}/commands')
        assert sorted(c['name'] for c in commands) == sorted(manifest['commands'])
        play = next(c for c in commands if c['name'] == 'play')
        assert not play['options'][0].get('required', False)
        if manifest.get('station_counts'):
            catalog = json.loads(run('docker', 'exec', CONTAINER, 'cat', '/app/catalog.json'))
            assert {s['id']: len(s['tracks']) for s in catalog['stations']} == manifest['station_counts']
            assert set(manifest['station_counts']) <= {c['value'] for c in play['options'][0]['choices']}
            for name, expected_hash in manifest['bundle_files'].items():
                assert run('docker', 'exec', CONTAINER, 'sha256sum', '/app/' + name).split()[0] == expected_hash
        legacy_commands = discord(env, f'applications/{app}/guilds/1524199248248176751/commands')
        legacy_names = {'play', 'station', 'nowplaying', 'pause', 'resume', 'skip', 'stop', 'skip-settings'}
        if manifest.get('remove_legacy_commands', False) and legacy_commands and {c['name'] for c in legacy_commands} <= legacy_names:
            (backup / 'legacy-guild-commands.json').write_text(json.dumps(legacy_commands, indent=2))
            discord(env, f'applications/{app}/guilds/1524199248248176751/commands', 'PUT', [])
        assert siblings() == before_siblings
        with closing(sqlite3.connect(ROOT / 'data/skip-settings.sqlite3')) as live, closing(sqlite3.connect(backup / 'after-cut.sqlite3')) as snapshot:
            live.backup(snapshot)
            assert snapshot.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        receipt = {**staged, 'binarySha256': manifest['sha256']['vozen-music'], 'catalogSha256': manifest['sha256']['catalog.json'], 'rollbackImage': previous, 'backup': str(backup), 'skipPoliciesPreserved': True, 'otherVozenContainersUnchanged': True, 'globalCommands': len(commands), 'active': True}
        (ROOT / 'expansion-release-20261031.json').write_text(json.dumps(receipt, indent=2))
        print(json.dumps(receipt))
    except Exception:
        subprocess.run(['systemctl', 'stop', 'vozen-music'], check=True)
        if (ROOT / 'data/skip-settings.sqlite3').exists():
            exported = run('docker', 'run', '--rm', '--network=none', '--read-only', '--cap-drop=ALL', '-v', str(ROOT / 'data') + ':/app/data', *cover_mount(), tag, '--export-skip')
            json.loads(exported)
            atomic(legacy_path, exported.encode())
            os.chown(legacy_path, 10001, 10001)
            with closing(sqlite3.connect(ROOT / 'data/skip-settings.sqlite3')) as db:
                for guild, body in db.execute('SELECT guild,body FROM playback').fetchall():
                    intent = json.loads(body)
                    intent['active'] = False
                    db.execute('UPDATE playback SET body=? WHERE guild=?', (json.dumps(intent), guild))
                db.commit()
        set_image(previous)
        if compose_path.read_bytes() == candidate_compose:
            atomic(compose_path, compose_bytes)
        subprocess.run(['systemctl', 'start', 'vozen-music'], check=True)
        healthy()
        raise

if __name__ == '__main__':
    main()
