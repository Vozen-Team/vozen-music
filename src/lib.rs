use serde::Deserialize;
use sha2::Digest;
pub mod expansion;
pub mod skip;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub fn check_application_identity(expected: u64, actual: u64) -> Result<(), Error> {
    if expected == 0 || expected != actual {
        return Err("The token does not belong to the configured Music application. No commands were changed.".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub stations: Vec<Station>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Station {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    #[serde(default)]
    pub id: String,
    pub title: String,
    pub artist: String,
    pub file: PathBuf,
    #[serde(default)]
    pub cover: Option<PathBuf>,
    pub source_url: String,
    pub license: String,
    pub license_evidence: String,
    pub instrumental: bool,
    pub instrumental_evidence: String,
    pub approved: bool,
    #[serde(default)]
    pub gain_db: f32,
}

impl Catalog {
    pub fn load(root: &Path) -> Result<Self, Error> {
        let mut catalog: Self = serde_json::from_slice(&fs::read(root.join("catalog.json"))?)?;
        let media = root.join("media").canonicalize()?;
        let mut ids = HashSet::new();
        if catalog.stations.is_empty() || catalog.stations.len() > 25 {
            return Err("The catalog must contain between 1 and 25 stations.".into());
        }
        for station in &mut catalog.stations {
            if station.id.is_empty()
                || station.id.len() > 32
                || !station
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                || !ids.insert(station.id.clone())
                || station.name.trim().is_empty()
                || station.name.chars().count() > 100
            {
                return Err("Invalid station or duplicate station ID.".into());
            }
            for track in &mut station.tracks {
                if track.id.is_empty() {
                    track.id = format!(
                        "rec-{:x}",
                        sha2::Sha256::digest(track.source_url.as_bytes())
                    );
                }
                // shortcut: initial catalog is CC0-only; review licenses before accepting other types.
                if !track.approved
                    || track.license != "CC0-1.0"
                    || !track.source_url.starts_with("https://")
                    || track.source_url.chars().count() > 800
                    || track.license_evidence.trim().is_empty()
                    || track.title.trim().is_empty()
                    || track.title.chars().count() > 200
                    || track.artist.trim().is_empty()
                    || track.artist.chars().count() > 200
                    || track.file.is_absolute()
                {
                    return Err(
                        "Invalid track: metadata or approval of its source and CC0 license is missing."
                            .into(),
                    );
                }
                if !track.instrumental || track.instrumental_evidence.trim().is_empty() {
                    return Err(
                        "Only approved instrumental tracks with source evidence are allowed."
                            .into(),
                    );
                }
                if !track.gain_db.is_finite() || !(-30.0..=20.0).contains(&track.gain_db) {
                    return Err("Track gain must be between -30 and 20 dB.".into());
                }
                let file = media.join(&track.file).canonicalize()?;
                if !file.starts_with(&media)
                    || !file.is_file()
                    || !matches!(
                        file.extension().and_then(|e| e.to_str()),
                        Some("mp3" | "wav" | "ogg" | "flac")
                    )
                {
                    return Err("The track must be an audio file inside media/.".into());
                }
                track.file = file;
                if let Some(cover) = &mut track.cover {
                    let folder = root.join("covers").canonicalize()?;
                    let file = root.join(&*cover).canonicalize()?;
                    if cover.is_absolute() || !file.starts_with(&folder) || !file.is_file() {
                        return Err("Track covers must be files inside covers/.".into());
                    }
                    if fs::metadata(&file)?.len() > 8 * 1024 * 1024 {
                        return Err("Track covers must be no larger than 8 MB.".into());
                    }
                    let bytes = fs::read(&file)?;
                    let name = format!("{:x}.png", sha2::Sha256::digest(&bytes));
                    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                        || file.file_name().and_then(|s| s.to_str()) != Some(&name)
                    {
                        return Err("Track covers must be PNGs named by their SHA-256.".into());
                    }
                    *cover = file;
                }
            }
        }
        Ok(catalog)
    }

    pub fn require_audio(&self) -> Result<(), Error> {
        if self.stations.iter().any(|s| s.tracks.is_empty()) {
            return Err(
                "Some stations have no approved audio. Complete the catalog before starting the bot."
                    .into(),
            );
        }
        Ok(())
    }
}

pub fn next_track(current: usize, count: usize) -> Option<usize> {
    if count == 0 || current >= count {
        None
    } else {
        Some((current + 1) % count)
    }
}

pub fn same_voice_channel(member: Option<u64>, bot: Option<u64>) -> bool {
    member.is_some() && (bot.is_none() || member == bot)
}

pub fn next_after_playback(
    current: usize,
    count: usize,
    consecutive_errors: usize,
    failed: bool,
) -> Option<(usize, usize)> {
    let errors = if failed {
        consecutive_errors.saturating_add(1)
    } else {
        0
    };
    if errors >= count {
        return None;
    }
    next_track(current, count).map(|index| (index, errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn rejects_a_token_for_a_different_application() {
        assert!(check_application_identity(123, 123).is_ok());
        assert!(check_application_identity(123, 456).is_err());
        assert!(check_application_identity(0, 0).is_err());
    }

    #[test]
    fn bundled_stations_have_playable_paths_and_instrumental_evidence() {
        let catalog = Catalog::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        catalog.require_audio().unwrap();
        assert_eq!(catalog.stations.len(), 16);
        for id in ["night", "rain", "sleep"] {
            let station = catalog.stations.iter().find(|s| s.id == id).unwrap();
            assert_eq!(station.tracks.len(), 25);
        }
        for station in &catalog.stations {
            assert!(
                !station.tracks.is_empty(),
                "Station {} has no tracks",
                station.id
            );
            for track in &station.tracks {
                assert!(track.instrumental);
                assert!(!track.instrumental_evidence.trim().is_empty());
                assert!(track.file.is_file());
            }
        }
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root =
                std::env::temp_dir().join(format!("vozen-music-{}-{unique}", std::process::id()));
            fs::create_dir_all(root.join("media")).unwrap();
            fs::write(
                root.join("media/test.wav"),
                b"test fixture, not playable audio",
            )
            .unwrap();
            Self(root)
        }
        fn load(&self, value: Value) -> Result<Catalog, Error> {
            fs::write(
                self.0.join("catalog.json"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
            Catalog::load(&self.0)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn catalog() -> Value {
        json!({"stations":[{"id":"lofi","name":"Vozen Lofi","tracks":[{
            "title":"Test", "artist":"Fixture", "file":"test.wav",
            "source_url":"https://example.com/test", "license":"CC0-1.0",
            "license_evidence":"Synthetic test metadata, no rights claim", "approved":true,
            "instrumental":true, "instrumental_evidence":"Synthetic instrumental test fixture"
        }]}]})
    }
    #[test]
    fn validates_local_catalog_and_wraps_station() {
        let fixture = Fixture::new();
        let loaded = fixture.load(catalog()).unwrap();
        loaded.require_audio().unwrap();
        assert!(loaded.stations[0].tracks[0].file.is_absolute());
        assert_eq!(next_track(0, 3), Some(1));
        assert_eq!(next_track(2, 3), Some(0));
        assert_eq!(next_track(0, 0), None);
        assert_eq!(next_track(3, 3), None);
    }

    #[test]
    fn covers_require_local_pngs_with_matching_content_hashes() {
        let fixture = Fixture::new();
        assert!(
            fixture.load(catalog()).unwrap().stations[0].tracks[0]
                .cover
                .is_none()
        );
        fs::create_dir(fixture.0.join("covers")).unwrap();
        let temporary = fixture.0.join("covers/test.png");
        image::RgbaImage::new(1, 1).save(&temporary).unwrap();
        let bytes = fs::read(&temporary).unwrap();
        let name = format!("{:x}.png", sha2::Sha256::digest(&bytes));
        let path = fixture.0.join("covers").join(&name);
        fs::rename(&temporary, &path).unwrap();
        let mut input = catalog();
        input["stations"][0]["tracks"][0]["cover"] = json!(format!("covers/{name}"));
        assert!(
            fixture.load(input.clone()).unwrap().stations[0].tracks[0]
                .cover
                .as_ref()
                .unwrap()
                .is_absolute()
        );
        for invalid in [
            "covers/missing.png".to_owned(),
            path.to_string_lossy().into_owned(),
            "media/test.wav".to_owned(),
        ] {
            let mut invalid_input = input.clone();
            invalid_input["stations"][0]["tracks"][0]["cover"] = json!(invalid);
            assert!(fixture.load(invalid_input).is_err());
        }
        fs::write(&path, b"not the approved image").unwrap();
        assert!(fixture.load(input).is_err());
    }
    #[test]
    fn rejects_missing_approval_and_other_licenses() {
        let fixture = Fixture::new();
        for (key, value) in [
            ("approved", json!(false)),
            ("license", json!("royalty-free")),
            ("license_evidence", json!("")),
        ] {
            let mut input = catalog();
            input["stations"][0]["tracks"][0][key] = value;
            assert!(fixture.load(input).is_err());
        }
    }
    #[test]
    fn rejects_vocal_or_undocumented_tracks() {
        let fixture = Fixture::new();
        for (key, value) in [
            ("instrumental", json!(false)),
            ("instrumental_evidence", json!("")),
        ] {
            let mut input = catalog();
            input["stations"][0]["tracks"][0][key] = value;
            assert!(fixture.load(input).is_err());
        }
        let mut input = catalog();
        input["stations"][0]["tracks"][0]
            .as_object_mut()
            .unwrap()
            .remove("instrumental");
        assert!(fixture.load(input).is_err());
    }

    #[test]
    fn skips_failed_tracks_but_stops_after_one_failed_station_cycle() {
        assert_eq!(next_after_playback(0, 3, 0, true), Some((1, 1)));
        assert_eq!(next_after_playback(1, 3, 1, true), Some((2, 2)));
        assert_eq!(next_after_playback(2, 3, 2, true), None);
        assert_eq!(next_after_playback(2, 3, 2, false), Some((0, 0)));
        assert_eq!(next_after_playback(0, 1, 0, true), None);
        assert_eq!(next_after_playback(0, 0, 0, false), None);
        assert_eq!(next_after_playback(0, 3, usize::MAX, true), None);
    }

    #[test]
    fn validates_playback_gain_and_keeps_legacy_catalogs_compatible() {
        let fixture = Fixture::new();
        assert_eq!(
            fixture.load(catalog()).unwrap().stations[0].tracks[0].gain_db,
            0.0
        );
        for gain in [-31.0, 20.1] {
            let mut input = catalog();
            input["stations"][0]["tracks"][0]["gain_db"] = json!(gain);
            assert!(fixture.load(input).is_err());
        }
    }
    #[test]
    fn rejects_escape_missing_audio_and_duplicate_stations() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("outside.wav"), b"test").unwrap();
        for path in ["../outside.wav", "missing.wav"] {
            let mut input = catalog();
            input["stations"][0]["tracks"][0]["file"] = json!(path);
            assert!(fixture.load(input).is_err());
        }
        let mut input = catalog();
        input["stations"] = json!([input["stations"][0], input["stations"][0]]);
        assert!(fixture.load(input).is_err());
    }
    #[test]
    fn empty_station_can_be_drafted_but_cannot_start() {
        let fixture = Fixture::new();
        let input = json!({"stations":[{"id":"lofi","name":"Vozen Lofi","tracks":[]}]});
        assert!(fixture.load(input).unwrap().require_audio().is_err());
    }
    #[test]
    fn playback_controls_require_same_voice_channel() {
        assert!(same_voice_channel(Some(10), None));
        assert!(same_voice_channel(Some(10), Some(10)));
        assert!(!same_voice_channel(None, None));
        assert!(!same_voice_channel(None, Some(10)));
        assert!(!same_voice_channel(Some(11), Some(10)));
    }
}
