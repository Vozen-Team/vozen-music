use std::{fs::File, io::ErrorKind, path::Path};
use symphonia::core::{
    codecs::DecoderOptions, errors::Error, formats::FormatOptions, io::MediaSourceStream,
    meta::MetadataOptions, probe::Hint,
};
use vozen_music::Catalog;

#[test]
fn every_bundled_recording_decodes_with_the_playback_codec_library() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let catalog = Catalog::load(root).unwrap();
    let records: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("licenses/catalog-sources.json")).unwrap())
            .unwrap();
    for station in catalog.stations {
        for track in station.tracks {
            let source = MediaSourceStream::new(
                Box::new(File::open(&track.file).unwrap()),
                Default::default(),
            );
            let mut hint = Hint::new();
            hint.with_extension(track.file.extension().unwrap().to_str().unwrap());
            let mut format = symphonia::default::get_probe()
                .format(
                    &hint,
                    source,
                    &FormatOptions::default(),
                    &MetadataOptions::default(),
                )
                .unwrap()
                .format;
            let stream = format.default_track().unwrap();
            let id = stream.id;
            let mut decoder = symphonia::default::get_codecs()
                .make(&stream.codec_params, &DecoderOptions::default())
                .unwrap();
            let mut seconds = 0.0;
            loop {
                let packet = match format.next_packet() {
                    Ok(packet) => packet,
                    Err(Error::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => break,
                    Err(error) => panic!("{}: packet error: {error}", track.title),
                };
                if packet.track_id() != id {
                    continue;
                }
                let audio = decoder
                    .decode(&packet)
                    .unwrap_or_else(|error| panic!("{}: decode error: {error}", track.title));
                assert!(
                    audio.spec().rate >= 44100,
                    "{}: low sample rate",
                    track.title
                );
                seconds += audio.frames() as f64 / f64::from(audio.spec().rate);
            }
            let record = records["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["title"] == track.title && r["artist"] == track.artist)
                .unwrap();
            let expected = record["duration_seconds"].as_f64().unwrap();
            assert!(
                (seconds - expected).abs() < 0.3,
                "{}: decoded {seconds}s, expected {expected}s",
                track.title
            );
        }
    }
}
