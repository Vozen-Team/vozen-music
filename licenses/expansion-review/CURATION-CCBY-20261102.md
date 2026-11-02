# CC BY expansion review — 2 November 2026

The owner accepted CC BY after individual licence, credit and audio review, and subsequently accepted music declared as generated with AI under the same requirements. No candidate has been approved or activated. The 250 active CC0 recordings and their files remain unchanged.

| Station | Original checked CC0 candidates | Additional checked CC BY candidates | Local listening pool |
|---|---:|---:|---:|
| Synthwave | 30 | 0 | 30 |
| Acoustic | 20 | 9 | 29 |
| Chiptune | 29 | 0 | 29 |
| Chill House | 13 | 17 | 30 |
| Drum & Bass | 12 | 18 | 30 |
| Bossa Nova | 7 | 10 | 17 |

The 165 candidates total 470.37 minutes. This is a listening pool, not a completed selection of instrumental playlists. Source genre tags and instrumental labels remain claims to assess by listening. The 14 separately documented FMA Bossa candidates are not counted: their downloads await the site's explicit acknowledgement. Thirteen declare AI generation and one declares no AI generation. A flamenco track from the same album was excluded. A Silverman Sound Bossa lead was excluded because its page explicitly identifies female vocals.

## Evidence and remaining gates

- `curation-ccby-20261102.json` preserves exact source-page licence links, author fields, notice text/links, source hashes and file/archive provenance. The earlier CC0 manifest is preserved separately. No NC, ND or SA licence is admitted by this expansion.
- 54 new files passed complete strict FFmpeg decoding, sample-rate, loudness/peak and PCM uniqueness checks against each other, the earlier 111 candidates and the active catalog. One original FLAC without duration metadata is held outside the checked pool, with its original retained. No short recordings below 30 seconds occur in the added checked pool.
- Three Bossa recordings came from Kevin MacLeod's official Incompetech catalog: Bossa Antigua, BossaBossa and Casa Bossa Nova. The evidence preserves the official metadata, ISRCs, download hashes and supplied CC BY 4.0 attribution template.
- OGA attribution review remains pending where artist names, external guidelines, supplied notices or sample rights require clarification. Broad mixed-genre collections were not used merely to fill quotas. The number of distinct decoded files does not prove distinct compositions.
- Full human listening, no-voice assessment, subjective quality, genre suitability and final source/credit review remain pending for every candidate. Nothing is promoted from a technical pass alone.

## Local review and compatibility

Listening: http://localhost:4789/ . It reuses the existing tool and hash-keyed browser notes; source files and prior review notes are not reset. The combined local manifest contains only technically checked candidates. The original CC0 and expanded CC BY provenance manifests remain separate. Exporting review notes never publishes audio.

Local dashboard: http://localhost:4790/music/dashboard?server=1&page=stations . The six proposals remain non-playable. The local runtime now accepts CC0, CC BY 3.0 and CC BY 4.0 only, retaining the explicit approval/instrumental gates. CC BY requires supplied credit notices and changes. Public text-only credit pages contain recording metadata, not server settings or sessions. The player adds one compact attribution link only for CC BY; transport and rating controls remain unchanged.

Validation: 69 Rust tests and Clippy all-targets with warnings denied passed; four source-provenance tests passed. Existing station UI checks passed at 320, 360, 390, 430, 768, 900 and 1440 px. These checks do not prove live audio, legal ownership or the absence of vocals. No CC BY audio or new station has been deployed by this follow-up.
