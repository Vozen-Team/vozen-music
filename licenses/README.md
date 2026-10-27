# Development catalog: published sources and licenses

Checked on 2026-10-27. The active catalog has 175 recordings, 25 per station. Selected source pages showed a CC0 license field and download links. Every active recording is documented in [catalog-sources.json](catalog-sources.json), and the [catalog list](../CATALOG.md) links each recording to its source. `approved` records approval for this development catalog based on published statements, not independent certification of authorship or sample rights. No artist endorsement is implied.

The pages link to [CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/). Its terms and limitations apply. The references and hashes below identify the files and support recovery; hashes do not prove a license.

## Chills

- Listed artist: Holizna.
- Source: https://opengameart.org/content/chills
- Listed publication date: 2024-04-11.
- Observed license field: CC0.
- Description statement: “This music is public domain Cc0, So use it how you want!”
- Original file: https://opengameart.org/sites/default/files/01_holiznacc0_-_chills_0.mp3
- Included as `media/chills.mp3`, without transformation.
- Size: 6999565 bytes; ffprobe duration: 174.948 seconds.
- SHA256: `88cf8c964ea08b7eca071332ce57ebb33a09838f02cccda0ade8e792c6bc6e2c`.

## Lofi again

- Listed artist: omfgdude.
- Source: https://opengameart.org/content/lofi-again
- Listed publication date: 2019-02-26.
- Observed license field: CC0.
- Observed credit field: Omfgdude. Artist credit is retained in the catalog and the track-info command.
- Original file: https://opengameart.org/sites/default/files/lofiagain.ogg
- Included as `media/lofi-again.ogg`, without transformation.
- Size: 2419417 bytes; ffprobe duration: 87.809274 seconds.
- SHA256: `d3b3410a186d45dadbcf87a8072b5a7f3b3f5a11fe38d9a7e9bdb3f66d5d86fa`.

## Included recordings and recovery

These two recordings are included in the repository; other local media files are ignored. If recovery is needed, download from the original URLs to the paths above and compare SHA256 before use. Future source availability has not been tested.

Both files passed full FFmpeg decoding on 2026-10-27. This establishes FFmpeg readability; that check alone does not establish Songbird playback or Discord audibility. Live development-server playback was separately confirmed by the owner.

## Expanded instrumental selection — 2026-10-27

The initial genre expansion contained 21 recordings in seven editorial categories. Twenty imported recordings were documented individually in [catalog-sources.json](catalog-sources.json), with published license, source URL, original download URL, original and included-file hashes, duration, and the evidence used for instrumental classification. Lofi again is now also included in that machine-readable register; its source describes instrumental tracks.

The six HoliznaCC0 recordings come from the author's [Chillout Instrumentals collection](https://opengameart.org/content/chillout-instrumentals-collection), whose individual upload page publishes CC0 and an Instrumental tag. The uploaded ZIP is an author-published asset pack with its own license, rather than a third-party collection of unrelated links. Its member filenames and archive hash are recorded for recovery.

WAV versions of Echoes of the Past, Col Legno, and Love Song were converted to Ogg Vorbis using FFmpeg with quality 5; original hashes and transformation details are recorded. Other recordings were not transformed. All 20 imported files passed complete FFmpeg decoding.

Sources with explicit Instrumental labels were preferred. Solo-piano descriptions and synthesis-only creation descriptions were also accepted as documented curation inferences. No independent full listening review or vocal detection was performed. These source labels do not prove absence of sung/spoken samples, vocal chops, or synthetic choirs. The choir-tagged Heavenly Loop candidate was excluded; Chills was removed from active playback for insufficient instrumental evidence and remains only as a legacy source record/file.

Osare (Unrest Theme) offers both OGA-BY 3.0 and CC0 on its source page; the CC0 option was selected. The catalog retains the named composer credit. Genre assignments are editorial, not a claim that every piece is a pure example of one genre.

## Expansion to 25 recordings per station — 2026-10-27

154 recordings were added, bringing the total to 175 recordings and approximately 404 minutes. Every selected source page offers CC0 1.0, including pages with multiple license options. No Spotify, YouTube, Lofi Bot audio, or music licensed only for noncommercial use was imported.

Additional lofi recordings include the author's Chill Beats and Relaxing Instrumentals collections. Only the MP3 member of each composition was imported; the alternative OGG encoding was not counted again. The four distinct atmospheric recordings from Atmospheric Puzzles are credited to legoluft; their menu arrangements were excluded from the count. The Rock station includes distinct arrangements from Umplix's CC0 Remakes submission, whose source declares Instrumental and CC0 and credits its original contributors. The corresponding original versions were not added as additional recordings. Category placement remains editorial and includes game music, synthetic rock, metal, jazz fusion, and short loops/cues.

New recordings were transcoded in full to Ogg Vorbis quality 5. Audio timestamps were regenerated from decoded sample order where necessary to avoid non-monotonic source timestamps. No trimming, repetition, stem extraction, or vocal removal was performed. The register retains the original recording hash, included-file hash, decoded stereo 48 kHz PCM hash, size, duration, download URL, archive/member details where applicable, published license links, source-page hash, source credits, and instrumental evidence. The original audio downloads and retrieved pages are retained in the local research directory outside this repository.

**No independent full listening audit or vocal detection has been performed.** Explicit Instrumental labels were preferred; solo-instrument descriptions and other instrument/performance/synthesis metadata were also used. The latter are recorded as source descriptions or curation inferences. They do not certify absence of vocal chops, speech, singing, or synthetic choirs. Candidates with declared vocals, chanting, vocoders, choirs, or unclear licensing were excluded. A strict no-voice requirement remains incompletely validated until listening review. CC0 source statements do not independently establish that every uploader controlled all composition, recording, and sample rights.

The local `scripts/check_catalog.py` checks all active files, minimum station counts, metadata/provenance agreement, file hashes, complete decoding, and duplicate decoded audio. It does not inspect vocal content or authenticate copyright ownership.

## Objective quality review — 2026-10-27

Seven recordings below 44.1 kHz and the anomalous Krank Water recording were replaced with other source-reviewed CC0 recordings. The catalog still contains 25 recordings per station. Thirty-six trailing-silence regions of at least three seconds were shortened, retaining a 0.25-second tail. Some original MP3 recordings were converted to Ogg Vorbis during this edit; source hashes are retained and included-file/decoded hashes and durations were updated. Rejected recordings and originals before tail edits are retained outside the repository in the local quality quarantine, and previous revisions retain the earlier files.

The active audio now totals approximately 396 minutes and 426 MB. [audio-quality.json](audio-quality.json) records full-file loudness/true-peak/silence measurements and their file hashes. Per-track gains target -18 LUFS with a projected -2 dBFS peak ceiling and a +20 dB maximum boost; projected levels are not measurements of Discord output. No full subjective listening review was performed. This quality review does not change the source-based limits of the instrumental or copyright review above.
