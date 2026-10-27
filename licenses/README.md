# Development catalog: published sources and licenses

Checked on 2026-10-27. Both individual source pages were opened and showed the author, a CC0 license field, and a download link. `approved` records approval for this development catalog based on those published statements, not independent certification of authorship or sample rights. No artist endorsement is implied.

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

The active catalog now includes 21 recordings in seven editorial categories. Twenty imported recordings are documented individually in [catalog-sources.json](catalog-sources.json), with published license, source URL, original download URL, original and included-file hashes, duration, and the evidence used for instrumental classification. Lofi again remains covered by its record above; its source describes instrumental tracks.

The six HoliznaCC0 recordings come from the author's [Chillout Instrumentals collection](https://opengameart.org/content/chillout-instrumentals-collection), whose individual upload page publishes CC0 and an Instrumental tag. The uploaded ZIP is an author-published asset pack with its own license, rather than a third-party collection of unrelated links. Its member filenames and archive hash are recorded for recovery.

WAV versions of Echoes of the Past, Col Legno, and Love Song were converted to Ogg Vorbis using FFmpeg with quality 5; original hashes and transformation details are recorded. Other recordings were not transformed. All 20 imported files passed complete FFmpeg decoding.

Sources with explicit Instrumental labels were preferred. Solo-piano descriptions and synthesis-only creation descriptions were also accepted as documented curation inferences. No independent full listening review or vocal detection was performed. These source labels do not prove absence of sung/spoken samples, vocal chops, or synthetic choirs. The choir-tagged Heavenly Loop candidate was excluded; Chills was removed from active playback for insufficient instrumental evidence and remains only as a legacy source record/file.

Osare (Unrest Theme) offers both OGA-BY 3.0 and CC0 on its source page; the CC0 option was selected. The catalog retains the named composer credit. Genre assignments are editorial, not a claim that every piece is a pure example of one genre.
