// Local UI fixture only: no Discord credentials, gateway or production data.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
const root = path.resolve(import.meta.dirname, "..");
const libraryRoot = process.env.VOZEN_PREVIEW_LIBRARY_ROOT || root;
let settings = {
  revision: 0,
  skip: {
    access: "everyone",
    mode: "instant",
    majority_scope: "call",
    moderator_roles: [],
  },
  volume: 100,
  default_station: "lofi",
  radio_mode: "session",
  voice_channel: null,
  voice_rules: { allow: false, channels: [] },
  text_rules: { allow: false, channels: [] },
  dj_roles: [],
  djs_only: false,
  player_locked: false,
  announcements: true,
  announcement_channel: null,
  tips: false,
  statistics: false,
};
let personal = {
  privacy: { enabled: false, visible: false },
  seconds: 0,
  ratings: {},
};
const durations = JSON.parse(fs.readFileSync(path.join(root, "web/track-durations.json"), "utf8"));
const stations = JSON.parse(
  fs.readFileSync(path.join(root, "catalog.json"), "utf8"),
).stations.map((s) => ({
  ...s,
  cover: `/music/albums/${s.id}-v1.webp`,
  tracks: s.tracks.map((t) => ({ ...t, source: t.source_url, license_url: ({"CC0-1.0":"https://creativecommons.org/publicdomain/zero/1.0/","CC-BY-3.0":"https://creativecommons.org/licenses/by/3.0/","CC-BY-4.0":"https://creativecommons.org/licenses/by/4.0/"})[t.license], duration_seconds: durations[t.id], thumbnail: `/music/thumbnails/${path.basename(t.cover, ".png")}.webp` })),
}));
stations.push({ id: "my", name: "My Station", tracks: [] });
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://localhost"),
    p = url.pathname;
  const json = (data, status = 200) => {
    res.writeHead(status, {
      "Content-Type": "application/json",
      "Cache-Control": "no-store",
    });
    res.end(JSON.stringify(data));
  };
  if (p === "/music/api/me")
    return json({
      csrf: "local-fixture",
      user: { id: "1", username: "Local preview" },
    });
  if (p === "/music/api/guilds")
    return json({ guilds: [{ id: "1", name: "Example server" }] });
  if (p === "/music/api/stations")
    return json({ stations, pending: [], proposals: JSON.parse(fs.readFileSync(path.join(root, "web/station-proposals.json"), "utf8")).filter(s => !stations.some(active => active.id === s.id)) });
  if (p.startsWith('/music/credits/')) {
    const track = stations.flatMap(s => s.tracks).find(t => t.id === decodeURIComponent(p.slice('/music/credits/'.length)));
    if (!track) return json({error:'Recording not found.'},404);
    res.writeHead(200, {'Content-Type':'text/plain; charset=utf-8','X-Content-Type-Options':'nosniff'});
    res.end(`${track.title}\nBy ${track.artist}\nSource: ${track.source}\nLicence: ${track.license}\n${track.license_url}\n${track.credit_notice || ''}\nChanges: ${track.changes || 'See source and catalog evidence.'}\n`);return;
  }
  if (p.startsWith("/music/api/recordings/") && p.endsWith("/audio")) {
    const id = p.split("/")[4];
    const track = stations.flatMap(s => s.tracks).find(t => t.id === id);
    if (!track) return json({ error: "Recording not found." }, 404);
    const file = path.join(libraryRoot, "media", track.file), size = fs.statSync(file).size;
    const match = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range || "");
    const start = match ? Number(match[1]) : 0, end = match && match[2] ? Math.min(Number(match[2]), size - 1) : size - 1;
    if (start > end || start >= size) { res.writeHead(416, { "Content-Range": `bytes */${size}` }); res.end(); return; }
    const type = { ".ogg": "audio/ogg", ".mp3": "audio/mpeg", ".wav": "audio/wav", ".flac": "audio/flac" }[path.extname(file)];
    res.writeHead(match ? 206 : 200, { "Content-Type": type, "Content-Length": end - start + 1, "Accept-Ranges": "bytes", ...(match ? { "Content-Range": `bytes ${start}-${end}/${size}` } : {}) });
    if (req.method === "HEAD") { res.end(); return; }
    const stream = fs.createReadStream(file, { start, end });
    res.on("close", () => stream.destroy()); stream.pipe(res); return;
  }
  // Layout fixture: original covers stand in for production's resized thumbnails.
  if (p.startsWith("/music/thumbnails/")) {
    const name = path.basename(p, ".webp") + ".png";
    const track = stations.flatMap(s => s.tracks).find(t => t.cover && path.basename(t.cover) === name);
    if (!track) { res.writeHead(404); res.end(); return; }
    res.setHeader("Content-Type", "image/png");
    res.end(fs.readFileSync(path.join(libraryRoot, track.cover))); return;
  }
  if (p.startsWith("/music/api/guilds/1/")) {
    const action = p.split("/").at(-1);
    let body = {};
    if (req.method !== "GET") {
      let text = "";
      for await (const chunk of req) {
        text += chunk;
        if (text.length > 6 * 1024 * 1024) {
          res.writeHead(413);
          res.end();
          return;
        }
      }
      try {
        body = JSON.parse(text);
      } catch {
        return json({ error: "Invalid JSON" }, 400);
      }
    }
    if (action === "server-settings") {
      if (req.method === "PUT") {
        if (body.revision !== settings.revision)
          return json(
            { error: "Settings changed. Reload before saving." },
            409,
          );
        if (body.volume < 0 || body.volume > 100)
          return json({ error: "Volume must be 0–100." }, 400);
        settings = { ...body, revision: settings.revision + 1 };
      }
      return json({
        settings,
        roles: [
          { id: "2", name: "DJ" },
          { id: "3", name: "Moderator" },
        ],
        channels: [
          { id: "4", name: "Lounge", voice: true },
          { id: "5", name: "music", voice: false },
        ],
        session: null,
      });
    }
    if (action === "personal") {
      if (body.privacy) personal.privacy = body.privacy;
      if (body.erase_statistics)
        personal = {
          ...personal,
          seconds: 0,
          privacy: { enabled: false, visible: false },
        };
      if (body.clear_ratings) personal.ratings = {};
      return json(personal);
    }
    if (action === "community") return json({ rows: [] });
    if (action === "playback")
      return json(
        {
          error:
            "Start Music in a voice channel first. This preview does not connect to Discord.",
        },
        409,
      );
    if (action === "profile")
      return req.method === "GET"
        ? json({ nick: "Vozen Music", bio: "Instrumental radio for your server.", default_name: "Vozen Music", default_avatar_url: "/music/avatar.png", bot_id: "1", policy: JSON.parse(fs.readFileSync(path.join(root, "web/profile-policy.json"), "utf8")) })
        : json(
            {
              error:
                "Profile changes require the real Discord dashboard. This page is a local UI fixture.",
            },
            400,
          );
  }
  if (p === "/music/auth/logout") return json({ saved: true });
  const mapping = {
    "/music/dashboard": "web/dashboard.html",
    "/music/": "web/dashboard.html",
    "/music/avatar.png": "assets/brand/v4/vozen-music-avatar.png",
    "/music/css/global-nav-v1.css": "web/global-nav-v1.css",
    "/music/header-font.woff2": "web/header-font.woff2",
    "/music/assets/fonts/unbounded-500-latin.woff2": "assets/fonts/unbounded-500-latin.woff2",
    "/music/assets/fonts/unbounded-500-latin-ext.woff2": "assets/fonts/unbounded-500-latin-ext.woff2",
  };
  const relative =
    mapping[p] ?? (p.startsWith("/music/") ? "web/" + p.slice(7) : "");
  const file = path.resolve(root, relative);
  if (
    !file.startsWith(root + path.sep) ||
    !fs.existsSync(file) ||
    !fs.statSync(file).isFile()
  ) {
    res.writeHead(404);
    res.end();
    return;
  }
  const types = {
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".png": "image/png",
    ".webp": "image/webp",
    ".svg": "image/svg+xml",
    ".woff2": "font/woff2",
  };
  res.setHeader(
    "Content-Type",
    types[path.extname(file)] || "application/octet-stream",
  );
  let bytes = fs.readFileSync(file);
  if (file.endsWith("dashboard.html"))
    bytes = Buffer.from(
      bytes
        .toString()
        .replace(
          '<main id="main" tabindex="-1">',
          '<main id="main" tabindex="-1"><p class="small">Local UI preview · sample server · no Discord actions</p>',
        ),
    );
  res.end(bytes);
});
const port = Number(process.env.VOZEN_PREVIEW_PORT || 4783);
server.listen(port, "127.0.0.1", () =>
  console.log(`Music UI fixture: http://localhost:${port}/music/dashboard`),
);
