# Vozen Music

Station-based instrumental radio bot for Discord, inspired by the format of [Lofi Bot](https://www.lofibot.xyz/). The active catalog contains 175 recordings across seven stations, selected from sources publishing CC0 licenses and instrumental evidence. Running locally in Docker; the original development playback was audibly confirmed by the owner on 2026-10-27. The expanded catalog has not been independently listened to in full. Not deployed to a VPS.

Visual identity: [brand guide](assets/brand/README.md), the selected vinyl-and-V concept adapted to Vozen's rounded outlines, cream, violet, blue, and cyan. The avatar has been applied to the Discord application.

## Language

**English is the default language for the entire bot.** Command names, option names, descriptions, responses, errors, logs, and setup text are English. The same English responses are used regardless of the member's Discord language. There are currently no translations or language-selection settings.

Station names and track credits come from the operator-controlled catalog. Keep station labels in English; preserve artists' names and original track titles. Internal historical design notes and concept artwork may retain Portuguese text; they are not runtime messages. Default English tagline for future product assets: **Find your rhythm.**

## Commands

| Command | Action |
| --- | --- |
| `/play station` | Join the member's voice channel and start a station. |
| `/station station` | Switch stations and start from the first track. |
| `/nowplaying` | Show the selected track, artist, source, and license. |
| `/pause` / `/resume` | Pause / resume playback. |
| `/skip` | Skip to the next track. |
| `/stop` | Stop playback and leave the voice channel. |

Tracks play in catalog order and repeat at the end. An audio error stops the station and attempts to post a notice in the command's text channel. Controls require membership in the bot's voice channel. Before the initial connection, the member must join a voice channel. Only standard voice channels are supported, not Stage channels.

Command replies are private; playback errors are posted in the command's text channel with mentions disabled. This version is limited to one development server.

## Catalog

| Station option | Display name | Active tracks |
| --- | --- | --- |
| `lofi` | Vozen Lofi | 25 |
| `ambient` | Vozen Ambient | 25 |
| `jazz` | Vozen Jazz | 25 |
| `electronic` | Vozen Electronic | 25 |
| `piano` | Vozen Piano | 25 |
| `cinematic` | Vozen Cinematic | 25 |
| `rock` | Vozen Rock | 25 |

Use `/station station:jazz`, for example. The catalog is not limited to lofi; station placement is editorial curation and some tracks blend styles. Rock includes metal and synthetic game-rock arrangements. Some recordings are short loops or cues. Total active playback is approximately 404 minutes across all seven stations; individual stations contain approximately 46–74 minutes before repetition. The [full catalog](CATALOG.md) lists every recording, artist, source, duration, and type of instrumental evidence.

Sources, hashes, durations, transformations, and instrumental evidence are recorded in [licenses/README.md](licenses/README.md) and [catalog-sources.json](licenses/catalog-sources.json). The approved recordings are included in the repository; other local audio files are ignored. The bot refuses to start if a station is empty or a track lacks explicit instrumental approval and evidence. Only the operator edits the catalog. This validates the operator's declaration, not the contents of an audio waveform.

Source evidence includes explicit Instrumental labels, solo-instrument descriptions, and documented synthesized compositions. Classifications based on instrument titles, tags, and creation descriptions are recorded as curation inferences, not independent listening results. No full listening audit or automated vocal detector was used. The strict no-voice requirement is therefore not fully validated. The desired policy excludes singing, speech, vocal chops, and choirs; flag and remove any recording found to contain these, even if its source labels it instrumental. Candidates with declared vocals, choirs, chanting, vocoders, or unclear licensing were excluded. Chills is retained only as a legacy recording and removed from active playback because its instrumental evidence was insufficient.

Run `python scripts/check_catalog.py` with FFmpeg available to check the minimum station size, complete provenance coverage, SHA256 hashes, full decoding, and duplicate decoded recordings. These checks do not detect voices. The expansion adds one selected encoding/arrangement per composition; alternative encodings and menu arrangements were not counted as additional tracks.

Place audio in `media/` and register tracks in `catalog.json`. Example schema, **not a real track or proof of a license**:

```json
{
  "title": "Track title",
  "artist": "Artist",
  "file": "track.mp3",
  "source_url": "https://original-artist.example/track",
  "license": "CC0-1.0",
  "license_evidence": "Archived license evidence for this recording, with date",
  "instrumental": true,
  "instrumental_evidence": "Source declaration or documented listening review supporting no vocals",
  "approved": false
}
```

Review the source and license for the composition, recording, and any samples before setting `approved: true`. Validation checks fields and paths; it does not authenticate rights claims. Preserve the license evidence outside temporary directories. This prototype only accepts `CC0-1.0`; review terms before adding other licenses. Lofi Bot's library license has not been verified and its audio is not used here.

## Run locally

Requires Rust and the native toolchain used by Songbird/Opus. Uses Serenity 0.12 and Songbird 0.6 with DAVE, without voice recording or the Message Content intent.

From this project directory:

```powershell
cargo test --all-targets --locked
cargo fmt --check
cargo run --locked -- --check
```

`--check` does not read tokens or contact Discord. It fails if a station is empty or a referenced file is missing. It does not decode audio; actual playback needs a separate test.

Set `DISCORD_TOKEN` for the **dedicated Music application**, its `DISCORD_APPLICATION_ID`, and a development `DISCORD_GUILD_ID` in the process environment. Before registering commands, the bot checks that the token belongs to the configured application. `.env.example` documents the fields; native `cargo run` does not load `.env` automatically. Do not reuse TTS/Helper tokens or commit secrets.

```powershell
cargo run --locked
```

This connects to Discord and **replaces this application's commands in the configured development server**. It does not register global commands. Invite Music using the `bot` and `applications.commands` scopes, with View Channel, Connect, and Speak permissions in the voice channel. Error notices also require View Channel and Send Messages in the text channel. Administrator is not required.

## Docker development setup

The dedicated Music application created on 2026-10-27 is `1564573935234973817`, in Voxi Team. Its profile icon and English description were verified in the Developer Portal. This does not prove the bot is installed or connected.

Create `.env` from `.env.example` if it does not exist, then fill in the three values locally. Use the token created by the owner in the Developer Portal. Never paste it into chat. `.env` is excluded from Git and the Docker build context.

```powershell
docker compose build
docker run --rm --network none --read-only vozen-music:local --check
docker compose up -d
docker compose logs --tail 30 music
```

Compose reads `.env` for this dedicated container. No inbound ports are published. The runtime uses an unprivileged user, a read-only filesystem, and dropped Linux capabilities. Catalog and audio are bundled in the image; rebuild after changing them. Docker SIGTERM and Ctrl-C request a gateway shutdown. Playback does not resume automatically after restart; use `/play station:lofi` again.

To stop only this project:

```powershell
docker compose down
```

The installation link with the four permissions listed above is:

[Install Vozen Music](https://discord.com/oauth2/authorize?client_id=1564573935234973817&scope=bot%20applications.commands&permissions=3148800&integration_type=0)

The owner chooses the development server and authorizes the installation. The configured `DISCORD_GUILD_ID` must match that server.

## Verification and limits

Local Linux compilation, tests, Clippy with `-D warnings`, and formatting were checked on 2026-10-27. The catalog check rejected the empty station and passed with both tracks. Both files passed full FFmpeg decoding; this does not prove Songbird playback or Discord audibility.

The native Windows build was blocked by Application Control, error 4551, when running a dependency build script. The policy was not changed. Local tests used the existing `vozen-qa-toolchain:20260930` Docker image. Docker build caches are isolated in the `vozen-music-registry` and `vozen-music-build` volumes.

The Music connected to the development server, and Discord's API confirmed all seven English guild commands. The owner confirmed that it joined the voice channel and played audible music on 2026-10-27. Natural track endings, station changes, pause, skip, stop, errors, and wrong-channel permissions still need real Discord checks. Unit tests include synthetic fixtures; they do not prove audibility. No VPS deployment has been performed. The bot currently depends on this computer and Docker Desktop remaining available.

The initial Docker release image compiled on 2026-10-27 and passed its offline catalog check as user `10001:10001` with a read-only filesystem and no network. The expanded-catalog revision has 11 unit tests, including rejection of undocumented/non-instrumental tracks and validation of all bundled paths. These checks do not prove the absence of vocals or live Discord audibility.

No Spotify/YouTube search, member-supplied URLs, web panel, payments, central entitlements, personalization, rain mixing, crossfade, or 24/7 restart recovery. One voice connection per server.
