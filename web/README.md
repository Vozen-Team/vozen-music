# Music dashboard

The dashboard is served by the Rust binary at `/music/dashboard`; there is no public marketing page in this release. English is the default throughout. Its controls configure the same per-server skip policy as `/skip-settings` and clear current votes on save.

Only servers with Music installed and where the signed-in member can manage the server are listed. Reads and writes additionally verify current membership and server permissions through Discord's bot API. Moderation permission alone does not grant dashboard access.

The dedicated application's confidential OAuth code flow requests only `identify guilds`. OAuth state is browser-bound, single-use and expires after 10 minutes. Access tokens remain in server memory; the browser receives an opaque HttpOnly, Secure, SameSite=Lax cookie scoped to `/music`. Sessions last at most one hour and are lost on restart. Writes and logout require both the exact configured Origin and a session CSRF token. Logout invalidates the local session and attempts Discord token revocation. No token is stored in browser localStorage or sent in API JSON.

The HTTP server uses a body limit of 8 KiB, a global budget of 600 requests/minute, at most 200 pending authorizations and 1000 active web sessions, and 12-second outbound timeouts. These are capacity bounds, not per-user quotas. Roles are transmitted as decimal strings to retain Discord ID precision. API failures leave the form editable for retry. Current settings are saved as full form values; concurrent administrators should reload before making further changes.

Colors and avatar follow the accepted Music identity in `assets/brand/README.md`. The locally hosted Outfit Latin font is reused from the Vozen website; its license is retained in `OFL-Outfit.txt`, sourced from https://github.com/google/fonts/tree/main/ofl/outfit . No external font or analytics requests are made.
