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

## Plan access candidate

Music reads the existing central resolver using signed, guild-scoped requests. Set both `MUSIC_ENTITLEMENT_URL` and `MUSIC_ENTITLEMENT_SECRET` to enable the gates. With neither configured, the current access rules apply. The bot does not process payments, provision seats, or read the billing database.

An active Plus member in a Music voice channel raises that call's target audio bitrate from 64 to 128 kbps, capped by the voice channel, and qualifies its starts for priority. Music rechecks membership and periodically refreshes access during playback. Server Premium qualifies independently of membership.

Set `MUSIC_TOPGG_TOKEN` in the protected environment after the Music project is listed on Top.gg. With entitlements and this project token enabled, Free managers need a current Top.gg vote to change the radio mode to 24/7 or Auto-join. Plus and Premium skip the Top.gg lookup. Returning to Session and server skip votes do not use this integration. Clearing the token disables only the Top.gg gate.

The production resolver listens on host loopback, outside the Music container. The optional socket and Compose override in this directory provide a private path. Validate the systemd proxy executable and the central listener, start `vozen-music-entitlement.socket` before the container, and include the override in Music's systemd Compose commands. Set `MUSIC_ENTITLEMENT_SOCKET=/run/vozen-music-entitlement/resolve.sock` and `MUSIC_ENTITLEMENT_URL=http://localhost/internal/v1/entitlements/resolve` in the protected environment. Keep the resolver on loopback and the socket out of the public proxy. Provision the signing secret through the existing host secret mechanism.

Validate the candidate in an isolated Music application before enabling it for existing guilds. Exercise an unlicensed server with a Free member and a Plus member, and a Premium server with the same Free member. Verify quota across restart and midnight UTC, Discord buttons and slash commands, settings writes, queue priority, and actual voice output. Compare audio at 64 and 128 kbps, allowing for the channel bitrate cap. Removing the entitlement environment variables restores the original access policy. Keep the database in place when rolling back. The new counter table does not overwrite existing settings.
