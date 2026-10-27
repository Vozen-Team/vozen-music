# VPS operation

The Music service is independent of TTS and Helper. Its Rust binary contains both the Discord runtime and the dashboard assets. The canonical source build is `docker build -t vozen-music:<revision> .`. Verify the source revision, tests, image identity and `--check` before starting it.

Production directory: `/srv/vozen-secure/music`, on the existing encrypted Vozen filesystem. Keep `.env` private to root (0600); set `DISCORD_TOKEN`, `DISCORD_APPLICATION_ID`, `DISCORD_CLIENT_SECRET`, `MUSIC_DASHBOARD_ORIGIN=https://api.vozen.org`, and the validated `MUSIC_IMAGE` tag. Register `https://api.vozen.org/music/auth/callback` in the dedicated Music application's Discord OAuth2 redirects. Do not reuse other bots' credentials.

Copy `compose.production.yaml` into this directory and create `data` with ownership `10001:10001`, mode 0700. Import any existing `skip-settings.json` only after stopping its previous writer. Do not run both local and VPS gateways for this application.

The production Compose file clears `DISCORD_GUILD_ID`, enabling global commands and all installed servers. Previously registered development guild commands may still override their matching global commands in that original guild. Both use the same handlers. The HTTP port is published only at `127.0.0.1:8790`. Keep it behind HTTPS.

Install `vozen-music.service` in `/etc/systemd/system`, reload systemd, and enable/start that service. Systemd owns restarts; container automatic restart is deliberately disabled so the service waits for the encrypted mount. The service is also wanted by Docker, allowing dependency restart ordering. Do not enable the development Compose project on the VPS.

The existing Caddy configuration uses its persisted JSON API configuration and `--resume`; its historical Caddyfile is not authoritative. Add a path-specific reverse proxy for `/music` and `/music/*` under the existing `api.vozen.org` host, preserving all other routes and TLS settings. Give this route `@id: vozen_music`, forward to `127.0.0.1:8790`, and use Caddy's Etag/If-Match protection when inserting it. Keep a private copy of the preceding config. Rollback removes only that route through `/id/vozen_music`; do not restore a whole old config over concurrent changes.

Acceptance: exact active image, `vozen-music.service` active, Discord gateway connected, `/music/health` returns 200 over HTTPS, unauthenticated settings return 401, OAuth redirect and secure cookies match the configured origin, and existing TTS/Helper health remains healthy. Test a real Discord login, a manager settings read/save, and persisted settings after a Music-only restart. Reboots and long-running voice playback require separate evidence.

Rollback: stop `vozen-music.service` before restarting the previously validated local Music container. Preserve the VPS `data` directory; copy newer settings back before resuming local use. The initial remote service has no previous production release. For later upgrades, retain the previous image and configuration and restore `MUSIC_IMAGE` before restarting the service. JSON settings currently have no migration. Playback and web sessions are not restored after a restart; users must sign in and run `/play` again.

This installation does not add Music to existing cloud backup jobs. Those jobs' existing scope is unchanged. Preserve a consistent private copy of the settings and credentials when extending the backup plan.
