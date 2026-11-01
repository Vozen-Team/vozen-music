# Track artwork

The catalog maps each recording to its own illustrated PNG. Filenames are the SHA-256 of the image bytes, so updated artwork gets a different URL. Original PNGs are retained separately; these binary assets are not committed to Git.

Before a fresh source build, restore the artwork bundle or fetch the published files from the existing Music API. From the repository root:

```python
import hashlib, json, urllib.request
from pathlib import Path

catalog = json.loads(Path('catalog.json').read_text(encoding='utf-8'))
for station in catalog['stations']:
    for track in station['tracks']:
        path = Path(track['cover'])
        data = path.read_bytes() if path.exists() else urllib.request.urlopen(
            'https://api.vozen.org/music/covers/' + path.name, timeout=30).read()
        assert hashlib.sha256(data).hexdigest() + '.png' == path.name
        path.parent.mkdir(exist_ok=True)
        path.write_bytes(data)
```

Production mounts `./covers` at `/app/covers` read-only. The player uses these public image URLs for normal stations and My Station; track credits and existing controls are preserved. Requests can read only catalogued images, not arbitrary files.

The existing release script accepts `MUSIC_RELEASE_INCOMING` and an optional `covers.tar`/`cover_files` manifest, validates all image hashes before staging, and checks the Compose change is limited to the read-only artwork mount. Rollback retains the original images and data, restores the previous Music image and Compose file, and does not overwrite newer SQLite writes.
