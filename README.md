# Vozen Music

Station-based instrumental radio bot for Discord, inspired by the format of [Lofi Bot](https://www.lofibot.xyz/). The active catalog contains 250 recordings across ten stations, selected from sources publishing CC0 licenses and instrumental evidence. Running on the VPS since 2026-10-27, with the previous local gateway stopped. The owner confirmed audible lofi playback from the VPS; the expanded catalog has not been independently listened to in full.

[Open the Music dashboard](https://api.vozen.org/music/dashboard) to configure skip rules through Discord login. There is no public Music marketing page in this release. See [dashboard behavior and security](web/README.md) and the [VPS operation guide](ops/README.md). Real Discord login, server settings read/save, persistence through a Music-only restart, reauthorization and restoration of the original settings were verified on 2026-10-27.

Visual identity: [brand guide](assets/brand/README.md), the selected vinyl-and-V concept adapted to Vozen's rounded outlines, cream, violet, blue, and cyan. The avatar has been applied to the Discord application.

## Language

**English is the default language for the entire bot.** Command names, option names, descriptions, responses, errors, logs, and setup text are English. The same English responses are used regardless of the member's Discord language. There are currently no translations or language-selection settings.

Station names and track credits come from the operator-controlled catalog. Keep station labels in English; preserve artists' names and original track titles. Internal historical design notes and concept artwork may retain Portuguese text; they are not runtime messages. Default English tagline for future product assets: **Find your rhythm.**

## Commands

| Command | Action |
| --- | --- |
| `/play [station]` | Join the member's voice channel; use the saved default when station is omitted. |
| `/station station` | Switch stations using the current listeners' preferences. |
| `/nowplaying` | Show the selected track, artist, source, and license. |
| `/pause` / `/resume` | Pause / resume playback. |
| `/skip` | Skip or vote to skip, using the same rules as the player button. |
| `/stop` | Stop playback and leave the voice channel. |
| `/skip-settings` | View or change this server's skip policy; requires Manage Server. |

Tracks are selected using the current listeners' ratings, with dislikes excluded and no immediate repeat when an alternative is available. My Station selects four favorite picks per discovery pick, with equal weight per person. A failed track is skipped automatically. If a whole station cycle fails consecutively, playback stops and attempts to post a notice in the command's text channel. Controls require membership in the bot's voice channel. Before the initial connection, the member must join a voice channel. Only standard voice channels are supported, not Stage channels.

Command replies are private. `/play` and `/station` also post a shared station player with the current recording and a Skip button. The player updates on track changes and shows vote progress; old controls are rejected. `/nowplaying` includes a private copy of the current button. Playback errors are posted in the command's text channel with mentions disabled. Production registers global commands and supports installed servers; the optional `DISCORD_GUILD_ID` restricts a development instance to one server.

The player puts Volume down, Previous, Pause/Resume, Next and Volume up in its first row as native emoji-only buttons, with a neutral heart, Repeat one and Leave underneath. Pending Previous/Next vote counts appear in the player status and private /nowplaying text, keeping transport buttons compact even during voting. Volume changes by ten percentage points per click, stays between 0 and 100, and disables its corresponding button at either limit. Leave stops playback and disconnects. The heart toggles only the clicking person's like for the current recording and server; its confirmation is private and the shared heart stays neutral. Slash `/like` remains idempotent, and `/dislike` remains available without a player button. Repeat one shows `↻ 1` and turns green when active. Other successful playback buttons remain silent; errors are private. Discord renders native button sizes and alignment rather than the localhost preview's CSS.

Previous uses up to 100 recordings from the current session's playback history and stays disabled until a previous recording exists. Previous and Next use the same skip access and majority policy, with independent votes; changing the track or skip policy clears both vote counts. Old controls remain bound to their session and recording generation.

The player image combines the original artwork with centered title/artist text on a plain dark background. Rust renders the PNG in memory using the bundled OFL-licensed Outfit fonts; no external image-generation service is involved. Long metadata wraps within the card, and its accessible image description retains title and artist. Playback state, station, volume and controls remain native Discord components. Same-track edits reuse the existing Discord image; a track change uploads its matching card. Original cover files and the public cover catalog remain unchanged. Legacy recordings without artwork retain native text metadata.

The shared player has no licensing footer. Recording sources and CC0 information remain available through the private `/nowplaying` response and **Stations → collection → Credits & licences** in the dashboard. Source evidence and bundled font licences remain in the release; only their placement in the shared player changed.

Repeat one turns green when enabled and repeats the current recording from its beginning after a normal ending. Manual Previous/Next keeps repeat enabled for the newly selected recording. Present listeners' dislikes still exclude recordings, and decode failures follow the normal bounded error handling instead of forcing a broken recording to repeat. History and repeat are session state: starting a new playback session or restarting Music resets them.

## Skip buttons and votes

Defaults are **everyone in the bot's voice channel**, **instant skip**, and **whole-call majority scope** (the scope only matters when voting is enabled). Buttons and `/skip` share the same permission and vote checks. Bots never vote or count in the quorum. Server managers and administrators must also be in the bot's voice channel to skip.

Configure with `/skip-settings`; omitted options keep their existing values, and no options shows the current policy. Only the owner or members with Manage Server/Administrator may use it. Examples:

```text
/skip-settings access:everyone mode:instant
/skip-settings access:everyone mode:majority majority-scope:call
/skip-settings access:moderators mode:majority majority-scope:eligible
/skip-settings access:moderators mode:majority majority-scope:call
/skip-settings add-role:@Moderator
/skip-settings remove-role:@Moderator
/skip-settings clear-roles:true
```

`access` is `everyone` or `moderators`; `mode` is `instant` or `majority`. `majority-scope:call` requires more than half of all humans currently in the call. `majority-scope:eligible` requires more than half of the humans allowed by the access policy. For example, ten humans with two eligible moderators require six votes under `call`, or two votes under `eligible`. A combination with too few eligible voters cannot pass; the voter receives a notice explaining this. Administrators do not bypass voting thresholds.

Server managers/administrators are always eligible under moderator access. With no configured moderator roles, moderation permissions (Moderate Members, Kick Members, Ban Members, or Manage Messages) qualify. Once custom roles are added, other moderators need at least one of those roles. `clear-roles:true` restores the permission-based definition; it also allows clearing roles that were deleted from Discord's role picker. Skip rules apply to skip requests; the other playback commands retain their existing same-voice-channel controls.

Votes are unique per user and track. On each request the bot rechecks current membership, excludes bots, and removes votes from members who left or are no longer eligible. Ineligible or unverifiable membership fails without skipping. Natural transitions, skips, station changes, stop, restart, and settings changes clear votes. Repeat voting requests from the same user have a two-second cooldown on the current track; instant skips have no such cooldown. A request already waiting for listener verification cannot vote on the next track.

Policies are saved transactionally per server in `data/skip-settings.sqlite3`. The old `data/skip-settings.json` is imported once and preserved; SQLite is the sole writer after migration. Invalid stored settings stop startup instead of silently opening access. A failed save retains the previous in-memory policy. Compose stores this file in its dedicated `music-data` volume; `/data/` is ignored by Git and excluded from the build context. Keep this volume to retain settings across recreations; `docker compose down -v` removes it. Votes are transient and are not saved. Only one Music process should write to this volume.

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

Use `/station station:jazz`, for example. The catalog is not limited to lofi; station placement is editorial curation and some tracks blend styles. Rock includes metal and synthetic game-rock arrangements. Some recordings are short loops or cues. Total active playback is approximately 585 minutes across all ten stations; individual stations contain approximately 43–74 minutes before repetition. The [full catalog](CATALOG.md) lists every recording, artist, source, duration, and type of instrumental evidence.

Sources, hashes, durations, transformations, and instrumental evidence are recorded in [licenses/README.md](licenses/README.md) and [catalog-sources.json](licenses/catalog-sources.json). The approved recordings are included in the repository; other local audio files are ignored. The bot refuses to start if a station is empty or a track lacks explicit instrumental approval and evidence. Only the operator edits the catalog. This validates the operator's declaration, not the contents of an audio waveform.

Source evidence includes explicit Instrumental labels, solo-instrument descriptions, and documented synthesized compositions. Classifications based on instrument titles, tags, and creation descriptions are recorded as curation inferences, not independent listening results. No full listening audit or automated vocal detector was used. The strict no-voice requirement is therefore not fully validated. The desired policy excludes singing, speech, vocal chops, and choirs; flag and remove any recording found to contain these, even if its source labels it instrumental. Candidates with declared vocals, choirs, chanting, vocoders, or unclear licensing were excluded. Chills is retained only as a legacy recording and removed from active playback because its instrumental evidence was insufficient.

Run `python scripts/check_catalog.py` with FFmpeg available to check the minimum station size, complete provenance coverage, SHA256 hashes, full decoding, and duplicate decoded recordings. These checks do not detect voices. The expansion adds one selected encoding/arrangement per composition; alternative encodings and menu arrangements were not counted as additional tracks.

## Audio quality and playback recovery

Objective review on 2026-10-27 replaced seven recordings below 44.1 kHz and one with an anomalous decoded peak. All 175 active recordings are now at least 44.1 kHz. Thirty-six recordings had trailing silence of at least three seconds shortened, retaining 0.25 seconds after the measured silence start. Original source hashes, transformations, and credits remain in the source register. These measurements do not certify subjective sound quality or absence of vocals.

[audio-quality.json](licenses/audio-quality.json) binds full-file EBU R128 measurements to each file's SHA256. The catalog's optional `gain_db` defaults to zero for older operator catalogs and is validated between -30 and +20 dB. Playback applies that gain before starting the track. Gains target approximately -18 LUFS, limited by a projected -2 dBFS true peak and a +20 dB boost cap. Projected levels are arithmetic estimates; Discord output has not been independently measured. The audio is not dynamically compressed at runtime.

Failed tracks advance once through a single End callback, which also receives Songbird's error state. Generation checks ignore stale callbacks, and callbacks are attached before playback starts. One complete cycle of consecutive failures stops the station rather than spinning indefinitely; a completed healthy track resets the failure count. Playback intent is persisted. Restart recovery revalidates channels and presence, restarting the saved recording from its beginning with at most three attempts. There is no guarantee of continuity through network or Discord outages. PC availability, network loss, Discord outages, and resource contention can still interrupt playback.

The codec integration test decodes every active recording using the same Symphonia version as Songbird and compares decoded duration to the source register. Run `cargo test --release --test audio_decode --locked` for the optimized full-catalog check. This is an offline codec test, not a Discord voice or long-running reliability test.

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

Set `DISCORD_TOKEN` for the **dedicated Music application**, its `DISCORD_APPLICATION_ID`, and optionally a development `DISCORD_GUILD_ID` in the process environment. Before registering commands, the bot checks that the token belongs to the configured application. `.env.example` documents the fields; native `cargo run` does not load `.env` automatically. Do not reuse TTS/Helper tokens or commit secrets. Do not start the local gateway while the VPS instance is running.

```powershell
cargo run --locked
```

This connects to Discord and **replaces this application's commands in the configured development server**, or its global commands when `DISCORD_GUILD_ID` is absent/empty. Invite Music using the `bot` and `applications.commands` scopes, with View Channel, Connect, and Speak permissions in the voice channel. Error notices also require View Channel and Send Messages in the text channel. Administrator is not required.

## Docker development setup

The dedicated Music application created on 2026-10-27 is `1564573935234973817`, in Voxi Team. Its profile icon and English description were verified in the Developer Portal. This does not prove the bot is installed or connected.

Create `.env` from `.env.example` if it does not exist, then fill in the three values locally. Use the token created by the owner in the Developer Portal. Never paste it into chat. `.env` is excluded from Git and the Docker build context.

```powershell
docker compose build
docker run --rm --network none --read-only vozen-music:local --check
docker compose up -d
docker compose logs --tail 30 music
```

Compose reads `.env` for this dedicated container. No inbound ports are published. The runtime uses an unprivileged user, a read-only filesystem, and dropped Linux capabilities. Catalog and audio are bundled in the image; rebuild after changing them. Docker SIGTERM and Ctrl-C request a gateway shutdown. Playback resumes from saved intent when permissions, radio mode and listener presence allow it. A stopped session stays stopped.

To stop only this project:

```powershell
docker compose down
```

The installation link with the four permissions listed above is:

[Install Vozen Music](https://discord.com/oauth2/authorize?client_id=1564573935234973817&scope=bot%20applications.commands&permissions=3165184&integration_type=0)

The owner chooses the development server and authorizes the installation. The configured `DISCORD_GUILD_ID` must match that server.

## Verification and limits

The dashboard release passed Linux release compilation, 31 unit/router tests, Clippy with `-D warnings`, and formatting on 2026-10-27. The earlier optimized codec integration test decoded all 175 active recordings with Symphonia and checked their durations; the earlier full FFmpeg/provenance/quality audit also passed. Audio and codec dependencies were unchanged by the dashboard release, so those full audio checks were not repeated. These offline checks do not establish Discord audibility or interruption-free operation.

The native Windows build was blocked by Application Control, error 4551, when running a dependency build script. The policy was not changed. Local tests used the existing `vozen-qa-toolchain:20260930` Docker image. Docker build caches are isolated in the `vozen-music-registry` and `vozen-music-build` volumes.

The Music connected to Discord from the VPS, and Discord's API confirmed all eight English global commands. The owner confirmed audible playback from both the original local instance and the VPS, using `/play station:lofi` for the VPS test. Natural track endings, station changes, pause, skip, stop, errors, wrong-channel permissions and multi-person voting still need real Discord checks. Unit tests include synthetic fixtures; they do not prove audibility. The VPS service is enabled with mount ordering and restart handling; no reboot test was performed. The expanded runtime supports conditional recovery; this requires its own real Discord acceptance checks.

The initial Docker release image compiled on 2026-10-27 and passed its offline catalog check as user `10001:10001` with a read-only filesystem and no network. The expanded-catalog revision has 11 unit tests, including rejection of undocumented/non-instrumental tracks and validation of all bundled paths. These checks do not prove the absence of vocals or live Discord audibility.

No Spotify/YouTube search, member-supplied URLs, payments, central entitlements, runtime rain mixing or crossfade. One voice connection per server. Premium, multiple calls and translations remain deferred. Night, Rain and Sleep each have 25 recordings admitted for owner-authorised live testing on 2026-10-31. Full human listening remains pending. Rain uses fixed offline CC0 mixes; there is no runtime mixer.

## Code license

Vozen Music source code is licensed under AGPL-3.0-only. See [LICENSE](LICENSE).

Audio recordings and other third-party material keep their own licenses and notices.
The recording licenses and attribution evidence remain in [licenses/README.md](licenses/README.md)
and [licenses/catalog-sources.json](licenses/catalog-sources.json). AGPL-3.0-only applies
to the project code; it does not replace those recording licenses.


## Expansion — 2026-10-31

The expanded implementation adds `/volume`, `/sleep`, `/like`, `/dislike`, `/library`, `/panel`, `/radio-mode`, `/settings`, `/profile`, `/leaderboard`, `/privacy`, `/help`, `/support`, `/invite` and `/feedback`. All features are free and English-only. The original eight commands remain supported.

The dashboard has Overview, Quick Setup, Playback, Access & Channels, Stations, Community and Bot Profile. It reuses the ecosystem account bridge. Playback settings include default volume/station, Session/24-7/Auto-join, DJ roles, player lock, channel rules, announcements and optional tips. Session leaves as soon as the last human leaves; bots do not count. Auto-join needs a fixed voice channel and a text channel. Explicit stop/sleep expiry prevents automatic restart until a new human entry or explicit play.

Stations presents the ten existing collections as album covers, with a separate My Station cover and explanation of its dynamic tracklist. Opening a collection lists its original recordings, artists, individual artwork and measured duration. Sources and licences are collected in a collapsed section after the tracklist. Hovering a row or focusing its number reveals Play; touch screens always show it. Previews play one track at a time in the browser, with pause/resume on the same button, and stop when leaving the page. They do not change Discord playback or contribute to listening statistics. Audio is streamed from catalog-listed recordings through a session-protected endpoint with byte-range support; nothing downloads before Play. Album links preserve the selected server and support direct loading and browser navigation. Eleven versioned 640px WebP covers live in `web/albums/`; their design prompts are in `assets/album-art-prompts.json`. Track durations in `web/track-durations.json` were matched to the SHA-256 measurements in `licenses/audio-quality.json` and verified against all 250 audio files. Original per-track artwork is unchanged; bounded background jobs create cached 128px WebP thumbnails, and only catalog-listed images can be requested.

Ratings and libraries belong to a person within one server. My Station is a virtual selection across the approved catalog, not an additional audio collection. Time-in-call statistics require server enablement and personal opt-in; visible rankings additionally require personal visibility. Collection excludes bots, paused playback and deafened time. No raw voice-event history, messages or audio is stored. Statistics and favorites have separate deletion actions.

Per-server nickname, avatar and banner updates go to Discord, never the global application profile. The description is read-only; applying or restoring the profile preserves it. Uploads accept static PNG/JPEG/WebP up to 2 MB and 4096 px, are decoded and re-encoded, and never fetch user-supplied URLs.

`node scripts/preview.mjs` opens a loopback-only UI fixture on port 4783. It uses sample data and does not connect to Discord or modify production. `scripts/check-ui.mjs` uses the existing Playwright installation via `VOZEN_PLAYWRIGHT_MODULE`. It is not a substitute for real voice tests.

See `docs/EXPANSION-ACCEPTANCE.md` for rollout, verification and unresolved music/listening gates. Unreviewed candidates under `licenses/expansion-review/` are not part of the active catalog.

The shared player is posted in the connected voice channel's text chat, both on `/play` (when automatic player announcements are enabled) and `/panel`. Track changes update that message. Music needs View Channel, Send Messages and Embed Links there; a posting failure is reported instead of silently posting in another channel. The configured Auto-join notification text channel remains available for Auto-join and its notices. Voice-chat controls follow voice allow/block rules and the fixed voice channel; ordinary text commands follow text allow/block rules. DJ, same-call and skip-voting checks still apply.

Bot Profile includes a live, approximate Discord-style preview of the display name, avatar and banner, with the existing description displayed as read-only text when present. Description edits are rejected by the API. Selected files stay in the browser until Apply; failed requests keep the editor values. The server validates profile text before image processing or calling Discord. The bundled profile-policy.json list rejects configured discriminatory slurs and hate slogans with common case, separator, width and leetspeak normalization; this is a maintained deterministic filter, not a comprehensive multilingual abuse classifier. Empty display names reset the server nickname to the bot's default. Existing image validation, manager-only access and CSRF checks remain required.

The shared player is deleted when its session stops or its voice connection leaves/moves. Reposting with /panel removes the previous player. Normal process shutdown also cleans up tracked player messages while keeping playback recovery intents. Deletion is serialized with player edits; an already-deleted message is harmless, and permission/network errors are logged. A forced process kill cannot perform Discord cleanup while the bot is offline.
