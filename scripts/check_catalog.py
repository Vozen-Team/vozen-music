"""Check the bundled catalog, provenance, file hashes, and full audio decoding."""

import hashlib
import json
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
catalog = json.loads((root / "catalog.json").read_text(encoding="utf8"))
records = json.loads((root / "licenses/catalog-sources.json").read_text(encoding="utf8"))["tracks"]
by_file = {record["file"]: record for record in records}
quality = json.loads((root / "licenses/audio-quality.json").read_text(encoding="utf8"))
by_quality = {record["file"]: record for record in quality["tracks"]}
assert len(by_file) == len(records), "Duplicate provenance records"
assert {s["id"] for s in catalog["stations"]} == {
    "lofi", "ambient", "jazz", "electronic", "piano", "cinematic", "rock"
}
seen_files, seen_audio = set(), set()
total_seconds = 0
for station in catalog["stations"]:
    assert len(station["tracks"]) >= 25, f"{station['id']}: fewer than 25 tracks"
    duration = 0
    for track in station["tracks"]:
        relative = "media/" + track["file"]
        path = (root / relative).resolve()
        assert path.is_relative_to(root / "media") and path.is_file(), relative
        assert relative not in seen_files, f"Repeated file: {relative}"
        seen_files.add(relative)
        record = by_file[relative]
        assert record["station"] == station["id"], relative
        assert record["title"] == track["title"] and record["artist"] == track["artist"], relative
        assert record["source_url"] == track["source_url"], relative
        assert track["approved"] and track["instrumental"] and track["instrumental_evidence"], relative
        assert track["license"] == record["selected_license"] == "CC0-1.0", relative
        assert record["sha256"] == hashlib.sha256(path.read_bytes()).hexdigest(), relative
        assert record["bytes"] == path.stat().st_size, relative
        measured = by_quality[track["file"]]
        assert measured["sha256"] == record["sha256"], relative
        assert measured["sample_rate"] >= 44100, relative
        assert abs(measured["gain_db"] - track["gain_db"]) < 0.000001, relative
        assert measured["projected_true_peak_dbfs"] <= -1.999999, relative
        assert -30 <= track["gain_db"] <= 20, relative
        result = subprocess.check_output([
            "ffmpeg", "-v", "error", "-xerror", "-nostdin", "-i", str(path),
            "-map", "0:a:0", "-ac", "2", "-ar", "48000", "-f", "hash", "-hash", "sha256", "-"
        ], text=True).strip().split("=")[1]
        assert result == record["pcm_sha256"], f"Decoded audio changed: {relative}"
        assert result not in seen_audio, f"Duplicate decoded recording: {relative}"
        seen_audio.add(result)
        duration += record["duration_seconds"]
    total_seconds += duration
    print(f"{station['id']}: {len(station['tracks'])} tracks, {duration / 60:.2f} minutes", flush=True)
assert seen_files == set(by_file), "Provenance and active catalog differ"
assert {path.removeprefix("media/") for path in seen_files} == set(by_quality), "Quality coverage differs"
print(f"Verified {len(seen_files)} files, {total_seconds / 60:.2f} minutes. Voice content was not audited.")
