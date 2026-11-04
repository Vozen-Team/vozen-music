"use strict";
const $ = (id) => document.getElementById(id);
let profileView = null;
let browserPreview = null;
function stopBrowserPreview() {
  if (!browserPreview) return;
  browserPreview.pause();
  browserPreview.removeAttribute("src");
  browserPreview.load();
  browserPreview = null;
}
let csrf = "",
  viewerId = "",
  current = "",
  saving = false,
  sequence = 0,
  serverList = [],
  model = null,
  baseline = "",
  draft = null,
  catalog = [],
  proposals = [],
  page = new URLSearchParams(location.search).get("page") || "overview";
const pages = {
  overview: "Overview",
  settings: "Settings",
  stations: "Stations",
  profile: "Bot Profile",
};
const pageIcons = {
  overview: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
  settings: "M4 7h16 M4 17h16 M8 4v6 M16 14v6",
  stations: "M3 10v4 M7 7v10 M12 4v16 M17 7v10 M21 10v4",
  profile: "M16 8a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z M4 21v-2a4 4 0 0 1 4-4h8a4 4 0 0 1 4 4v2",
};
const settingsSections = {
  playback: "Playback",
  access: "Access & Channels",
  community: "Community",
};
const descriptions = {
  overview: "Playback, stations and server settings.",
  settings: "Choose how Music plays and what members can do in this server.",
  stations: "Find an instrumental station for your mood.",
  profile: "Change how Music looks in this server only.",
};
function el(tag, text, cls) {
  const e = document.createElement(tag);
  if (text !== undefined) e.textContent = text;
  if (cls) e.className = cls;
  return e;
}
function notice(message, kind = "info", retry = null) {
  $("notice").textContent = message;
  $("notice").dataset.kind = kind;
  $("notice").hidden = !message;
  $("load-retry").hidden = !retry;
  $("load-retry").onclick = retry;
}
async function api(path, options = {}) {
  const read = !options.method || options.method.toUpperCase() === "GET";
  for (let attempt = 0; ; attempt++) {
    let r;
    try {
      r = await fetch(`/music${path}`, {
        credentials: "same-origin",
        ...options,
        headers: {
          ...options.headers,
          ...(options.method ? { "X-CSRF-Token": csrf } : {}),
        },
      });
    } catch {
      if (read && attempt < 2) {
        await new Promise(resolve => setTimeout(resolve, 500 * (attempt + 1)));
        continue;
      }
      throw Error(read ? "Could not connect to Music. Try again." : "The response was lost. Reload to check whether your change was applied.");
    }
    if (read && attempt < 2 && [502, 503, 504].includes(r.status)) {
      await r.body?.cancel();
      await new Promise(resolve => setTimeout(resolve, 500 * (attempt + 1)));
      continue;
    }
    if (r.status === 401) {
      signedOut();
      throw Object.assign(Error("Your session expired. Sign in again."), { status: 401 });
    }
    let data;
    try {
      data = await r.json();
    } catch {
      throw Object.assign(Error("The server could not respond. Try again."), { status: r.status });
    }
    if (!r.ok) {
      throw Object.assign(Error(data.error || "The request failed."), {
        status: r.status,
      });
    }
    return data;
  }
}
function showLoading(workspace) {
  $("loading").hidden = workspace !== false;
  $("workspace-loading").hidden = workspace !== true;
}
function signedOut() {
  stopBrowserPreview();
  csrf = "";
  current = "";
  sequence++;
  for (const id of ["editor", "sidebar", "loading", "workspace-loading", "empty", "server-picker"])
    $(id).hidden = true;
  $("welcome").hidden = false;
  renderAccount(null);
}
function changed() {
  return !!draft && JSON.stringify(draft) !== baseline;
}
function get(path) {
  return path.split(".").reduce((o, k) => o[k], draft);
}
function set(path, value) {
  const parts = path.split("."),
    key = parts.pop();
  parts.reduce((o, k) => o[k], draft)[key] = value;
  persistDraft();
  updateSave();
}
function validDraft(value, base) {
  if (
    !value ||
    typeof value !== "object" ||
    Object.keys(value).length !== Object.keys(base).length
  )
    return false;
  return Object.entries(base).every(([k, v]) => {
    const item = value[k];
    if (v === null)
      return item === null || (typeof item === "string" && /^\d+$/.test(item));
    if (Array.isArray(v))
      return (
        Array.isArray(item) &&
        item.every((i) => typeof i === "string" && /^\d+$/.test(i))
      );
    if (typeof v === "object") return validDraft(item, v);
    return (
      typeof item === typeof v &&
      (typeof item !== "number" || Number.isFinite(item))
    );
  });
}
function persistDraft() {
  try {
    if (changed())
      localStorage.setItem(`music-draft:${current}`, JSON.stringify(draft));
    else localStorage.removeItem(`music-draft:${current}`);
    return true;
  } catch {
    notice(
      "This browser could not preserve your draft. Keep this tab open until you save.",
      "error",
    );
    return false;
  }
}
function button(text, action, primary = false) {
  const b = el("button", text, primary ? "primary" : "");
  b.type = "button";
  b.addEventListener("click", () =>
    Promise.resolve(action()).catch((e) => notice(e.message, "error")),
  );
  return b;
}
function card(title, description, heading = "h2") {
  const c = el("section", undefined, "music-card");
  c.append(el(heading, title));
  if (description) c.append(el("p", description, "help"));
  return c;
}
function field(path, label, type = "text", options = [], help = "") {
  const wrap = el("div", undefined, "music-field"),
    id = `field-${path.replaceAll(".", "-")}`;
  const l = el("label", label);
  l.htmlFor = id;
  const copy = el("div", undefined, "settings-field-copy");
  copy.append(l);
  let input;
  if (type === "select") {
    input = el("select");
    for (const [value, text] of options) input.add(new Option(text, value));
    input.value = get(path) ?? "";
  } else if (type === "multi") {
    input = el("div", undefined, "resource-list");
    input.setAttribute("role", "group");
    input.setAttribute("aria-labelledby", `${id}-label`);
    l.id = `${id}-label`;
    const selected = get(path) || [];
    for (const [value, text] of options) {
      const row = el("label"),
        check = el("input");
      check.type = "checkbox";
      check.value = value;
      check.checked = selected.includes(value);
      check.addEventListener("change", () =>
        set(
          path,
          [...input.querySelectorAll("input:checked")].map((i) => i.value),
        ),
      );
      row.append(check, el("span", text));
      input.append(row);
    }
    if (!options.length) input.append(el("p", "No resources available."));
    const missing = selected.filter((id) => !options.some(([v]) => v === id));
    for (const value of missing) {
      const row = el("label"),
        check = el("input");
      check.type = "checkbox";
      check.checked = true;
      check.value = value;
      check.addEventListener("change", () =>
        set(
          path,
          [...input.querySelectorAll("input:checked")].map((i) => i.value),
        ),
      );
      row.append(
        check,
        el("span", `Unavailable resource (${value}) — uncheck to remove`),
      );
      input.append(row);
    }
  } else {
    input = el("input");
    input.type = type;
    if (type === "checkbox") {
      input.checked = !!get(path);
      input.setAttribute("role", "switch");
    } else input.value = get(path) ?? "";
    if (type === "number") {
      input.min = 0;
      input.max = 100;
      input.step = 1;
    }
  }
  input.id = id;
  if (type !== "multi")
    input.addEventListener(
      type === "checkbox" || type === "select" ? "change" : "input",
      () => {
        input.setCustomValidity("");
        if (
          type === "number" &&
          (!input.validity.valid || input.value === "")
        ) {
          input.setCustomValidity("Choose a whole number from 0 to 100.");
          return;
        }
        input.setCustomValidity("");
        let value =
          type === "checkbox"
            ? input.checked
            : type === "number"
              ? Number(input.value)
              : input.value;
        if (path.endsWith(".allow")) value = value === "true";
        set(
          path,
          ["voice_channel", "announcement_channel"].includes(path) && !value
            ? null
            : value,
        );
        dependent();
      },
    );
  if (help) {
    const hint = el("small", help);
    hint.id = `${id}-help`;
    input.setAttribute("aria-describedby", hint.id);
    copy.append(hint);
  }
  wrap.append(copy, input);
  return wrap;
}
function stationOptions() {
  return catalog.map((s) => [s.id, s.name]);
}
function channelOptions(voice) {
  return [
    ["", "None"],
    ...model.channels
      .filter((c) => c.voice === voice)
      .map((c) => [c.id, c.name]),
  ];
}
function roles() {
  return model.roles.map((r) => [r.id, r.name]);
}
function group(title, description, fields) {
  const c = card(title, description, "h3"),
    grid = el("div", undefined, "settings-fields");
  grid.append(...fields);
  c.append(grid);
  return c;
}
function dependent() {
  document.querySelectorAll("[data-condition]").forEach((n) => {
    const [key, value] = n.dataset.condition.split("=");
    n.hidden = String(get(key)) !== value;
  });
}
function conditional(node, condition) {
  node.dataset.condition = condition;
  return node;
}
function playbackFields() {
  return [
    field("default_station", "Default station", "select", stationOptions(),
      "Used when someone starts Music without picking a station."),
    field(
      "volume",
      "Default and current volume",
      "number",
      [],
      "Set the volume for the current player and future sessions, from 0 to 100.",
    ),
    field("radio_mode", "Radio mode", "select", [
      ["session", "Session · Leave when empty"],
      ["always", "24/7 · Stay connected"],
      ["auto_join", "Auto-join · Start on entry"],
    ]),
    field(
      "voice_channel",
      "Fixed voice channel",
      "select",
      channelOptions(true),
      "Limit playback to one voice channel. Required for Auto-join.",
    ),
  ];
}

function playerMessageFields() {
  return [
    field("announcements", "Show the player automatically", "checkbox", [],
      "Post the player in the voice channel’s chat."),
    field(
      "announcement_channel",
      "Auto-join notification channel",
      "select",
      channelOptions(false),
      "Choose where Auto-join posts its notification. Required in Auto-join mode.",
    ),
    field("tips", "Show command tips on the player", "checkbox"),
  ];
}
function accessFields() {
  return [
    field(
      "djs_only",
      "DJ-only controls",
      "checkbox",
      [],
      "Only DJs and server managers can control playback.",
    ),
    field("player_locked", "Lock the player", "checkbox", [],
      "Temporarily restrict playback controls to DJs and server managers."),
    field("dj_roles", "DJ roles", "multi", roles(), "Members with any of these roles are DJs."),
  ];
}

function skipFields() {
  return [
    field("skip.access", "Who can skip?", "select", [
      ["everyone", "Everyone in the call"],
      ["moderators", "Moderators only"],
    ]),
    field("skip.mode", "How does a skip happen?", "select", [
      ["instant", "One click"],
      ["majority", "Majority vote"],
    ], "Choose whether one person can skip or listeners need to vote."),
    conditional(
      field(
        "skip.majority_scope",
        "Who counts towards the majority?",
        "select",
        [
          ["call", "Everyone in the call"],
          ["eligible", "Only eligible voters"],
        ],
      ),
      "skip.mode=majority",
    ),
    conditional(
      field(
        "skip.moderator_roles",
        "Moderator roles",
        "multi",
        roles(),
        "Leave empty to use moderation permissions. Managers still need to be in the call and satisfy the vote threshold.",
      ),
      "skip.access=moderators",
    ),
  ];
}
function updateSave() {
  for (const c of document.querySelectorAll(
    "#editor input,#editor select,#editor button,#music-pages button",
  ))
    c.disabled = saving;
  const bar = $("music-savebar");
  if (!bar) return;
  bar.hidden = !changed() && !saving;
  $("music-save").disabled = saving || !changed();
  $("music-discard").disabled = saving;
  $("music-save-status").textContent = saving
    ? "Saving…"
    : "You have unsaved changes";
  $("server-select").disabled = saving;
}
async function save() {
  if (saving || !changed()) return;
  const invalid = $("editor").querySelector(".music-field :invalid");
  if (invalid) {
    const panel = invalid.closest(".settings-section[hidden]");
    if (panel) navigate(panel.dataset.settingsSection);
    invalid.reportValidity();
    return;
  }
  saving = true;
  updateSave();
  try {
    const result = await api(`/api/guilds/${current}/server-settings`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(draft),
    });
    draft = structuredClone(result.settings);
    baseline = JSON.stringify(draft);
    model.settings = draft;
    persistDraft();
    notice(
      result.warning
        ? `Settings saved for this server. ${result.warning}`
        : "Settings saved for this server.",
      result.warning ? "info" : "success",
    );
    render();
  } catch (e) {
    notice(e.message, "error");
  } finally {
    saving = false;
    updateSave();
  }
}
function requestedServer() {
  return /^\/music\/([1-9][0-9]*)$/.exec(location.pathname)?.[1]
    || new URLSearchParams(location.search).get("server");
}
function pageLocation(target) {
  const url = new URL(location.href);
  const section = Object.hasOwn(settingsSections, target);
  if (target === "overview") url.searchParams.delete("page");
  else url.searchParams.set("page", section ? "settings" : target);
  if (section) url.searchParams.set("section", target);
  else url.searchParams.delete("section");
  url.searchParams.delete("album");
  return url;
}
function showSettingsSection() {
  const nav = document.querySelector(".settings-jump");
  if (!nav) return;
  const section = new URLSearchParams(location.search).get("section");
  const active = Object.hasOwn(settingsSections, section) ? section : "playback";
  for (const link of nav.children) {
    const key = link.dataset.settingsSection;
    $(`settings-${key}`).parentElement.hidden = key !== active;
    if (key === active) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
}
function focusSettingsSection() {
  if (page !== "settings") return false;
  const section = new URLSearchParams(location.search).get("section");
  const heading = $("settings-" + section);
  if (!heading) return false;
  document.querySelector(".settings-jump").scrollIntoView({ block: "nearest" });
  heading.focus({ preventScroll: true });
  return true;
}
function navigate(next) {
  const url = pageLocation(next);
  const previous = page;
  page = url.searchParams.get("page") || "overview";
  history.replaceState(null, "", url);
  if (previous !== "settings" || page !== "settings") render();
  else showSettingsSection();
  closeSections(false);
  if (!focusSettingsSection()) {
    $("main").scrollTop = 0;
    $("main").focus();
  }
}
function stationLibrary(editor) {
  const stations = [...catalog.filter(s => s.id !== "my"), ...proposals.map(s => ({...s, proposed: true, tracks: []})), {
    id: "my", name: "My Station", cover: "/music/albums/my-v1.webp", tracks: [],
  }];
  const selected = stations.find(s => s.id === new URLSearchParams(location.search).get("album"));
  const linkTo = id => {
    const url = new URL(location.href);
    url.searchParams.set("page", "stations");
    if (id) url.searchParams.set("album", id); else url.searchParams.delete("album");
    return url.pathname + url.search;
  };
  const openAlbum = event => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    history.pushState(null, "", event.currentTarget.href);
    render();
    $("main").scrollTop = 0;
    $("main").focus();
  };
  const art = station => {
    const img = el("img");
    img.src = station.cover || "/music/avatar.png";
    img.alt = "";
    img.width = img.height = 640;
    img.loading = "lazy";
    return img;
  };
  if (!selected) {
    const library = el("div", undefined, "station-albums");
    const albumLink = station => {
      const album = el("a", undefined, "station-album");
      album.href = linkTo(station.id);
      album.addEventListener("click", openAlbum);
      return album;
    };
    const personal = stations.find(station => station.id === "my");
    const featured = albumLink(personal);
    featured.classList.add("station-featured");
    const cover = art(personal);
    cover.loading = "eager";
    const intro = el("div", undefined, "station-featured-copy");
    intro.append(el("span", "Personalized radio", "station-featured-label"), el("h2", personal.name), el("span", "A mix shaped by the people in your call."));
    const arrow = el("span", "→", "station-featured-arrow");
    arrow.setAttribute("aria-hidden", "true");
    featured.append(cover, intro, arrow);
    library.append(featured);

    const groups = [
      ["Slow down", ["lofi", "ambient", "night", "rain", "sleep"]],
      ["Jazz & acoustic", ["jazz", "piano", "acoustic", "bossa-nova"]],
      ["Electronic", ["electronic", "synthwave", "chill-house", "chiptune"]],
      ["Rock, scores & bass", ["rock", "cinematic", "drum-and-bass"]],
    ];
    const remaining = new Map(stations.filter(s => s.id !== "my").map(s => [s.id, s]));
    const addShelf = (title, entries) => {
      if (!entries.length) return;
      const shelf = el("section", undefined, "station-shelf");
      const grid = el("div", undefined, "station-shelf-grid");
      for (const station of entries) {
        const album = albumLink(station);
        album.append(art(station), el("strong", station.name), el("span", station.proposed ? "In preparation" : `${station.tracks.length} tracks`));
        grid.append(album);
      }
      shelf.append(el("h2", title), grid);
      library.append(shelf);
    };
    for (const [title, ids] of groups) {
      addShelf(title, ids.map(id => remaining.get(id)).filter(Boolean));
      ids.forEach(id => remaining.delete(id));
    }
    addShelf("More stations", [...remaining.values()]);
    editor.append(library);
    return;
  }
  const back = el("a", undefined, "button quiet album-back");
  const arrow = el("span", undefined, "album-back-icon");
  arrow.setAttribute("aria-hidden", "true");
  back.append(arrow, el("span", "All stations"));
  back.href = linkTo(null);
  back.addEventListener("click", openAlbum);
  const hero = el("section", undefined, "station-album-header");
  const info = el("div");
  const metadata = el("p", undefined, "album-metadata");
  if (selected.proposed) metadata.textContent = "In preparation · Not available for playback";
  else if (selected.id === "my") metadata.textContent = "Built from the listeners in your call.";
  else metadata.append(el("strong", `${selected.tracks.length} tracks`), el("span", "Various artists"));
  info.append(el("p", selected.id === "my" ? "Personalized radio" : "Instrumental radio", "album-label"), el("h1", selected.name), metadata);
  const cover = art(selected);
  cover.loading = "eager";
  hero.append(cover, back, info);
  editor.append(hero);
  if (selected.proposed) {
    editor.append(card(selected.name, selected.description));
    editor.append(card("Music selection in progress", "This station will open once its instrumental tracks have passed the listening and license checks."));
    return;
  }
  if (selected.id === "my") {
    editor.append(card("A station that follows your listeners", "Four favorite selections for every discovery selection, using the people currently in the call. Each person has equal weight; dislikes exclude recordings. Use /like, /dislike and /library. My Station has no fixed tracklist."));
    editor.append(card("Only from Music’s approved catalog", "My Station chooses from the instrumental recordings in the stations above. Like tracks to shape future selections, or dislike them to exclude them while you are in the call. Manage your ratings with /library. Uploads, external links and tracks outside the catalog are not accepted."));
    return;
  }
  const duration = seconds => Number.isFinite(seconds) && seconds >= 0
    ? `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}` : "—";
  const audio = el("audio");
  audio.preload = "none"; audio.volume = 0.5; audio.hidden = true;
  browserPreview = audio;
  const previewStatus = el("p", "Preview tracks in this browser. Discord playback stays unchanged.", "browser-preview-status");
  previewStatus.setAttribute("role", "status");
  let active = null, request = 0;
  const controls = new Map();
  const syncPreview = () => {
    for (const [id, { button: control, row, track }] of controls) {
      const playing = active?.id === id && !audio.paused && !audio.ended;
      control.classList.toggle("is-playing", playing);
      row.classList.toggle("is-previewing", playing);
      control.setAttribute("aria-pressed", String(playing));
      control.setAttribute("aria-label", `${playing ? "Pause" : "Play"} preview of ${track.title}`);
    }
    previewStatus.textContent = active ? `${audio.ended ? "Finished" : audio.paused ? "Paused" : "Previewing"}: ${active.title} — ${active.artist}. Only in this browser.` : "Preview tracks in this browser. Discord playback stays unchanged.";
  };
  for (const event of ["play", "pause", "ended", "emptied"]) audio.addEventListener(event, syncPreview);
  const playPreview = async track => {
    if (active?.id === track.id && !audio.paused) { audio.pause(); return; }
    const currentRequest = ++request;
    notice("");
    if (active?.id !== track.id || audio.error) {
      audio.pause(); active = track;
      audio.src = `/music/api/recordings/${encodeURIComponent(track.id)}/audio`;
    }
    try { await audio.play(); }
    catch (error) {
      if (currentRequest === request && audio.isConnected && error.name !== "AbortError") {
        audio.pause();
        notice("This track could not be previewed. Reload or try another track.", "error");
      }
    }
    syncPreview();
  };
  editor.append(audio, previewStatus);
  const table = el("table", undefined, "station-tracks");
  const caption = el("caption", `${selected.name} — tracks and credits`, "station-sr-only");
  const head = el("thead"), headings = el("tr"), body = el("tbody");
  for (const [label, cls] of [["#", "track-number"], ["Title", ""], ["Album", "track-album"], ["Duration", "track-duration"]]) {
    const th = el("th", label, cls);
    th.scope = "col"; headings.append(th);
  }
  head.append(headings);
  selected.tracks.forEach((track, index) => {
    const row = el("tr");
    const position = el("td", undefined, "track-number");
    const play = button("", () => playPreview(track));
    play.className = "track-preview";
    play.setAttribute("aria-label", `Play preview of ${track.title}`);
    play.setAttribute("aria-pressed", "false");
    for (const [text, cls] of [[String(index + 1), "preview-number"], ["▶︎", "preview-play"], ["⏸︎", "preview-pause"]]) {
      const icon = el("span", text, cls); icon.setAttribute("aria-hidden", "true"); play.append(icon);
    }
    controls.set(track.id, { button: play, row, track });
    position.append(play); row.append(position);
    const cell = el("td"), identity = el("div", undefined, "track-identity"), text = el("div", undefined, "track-text");
    const thumb = el("img");
    thumb.src = track.thumbnail || "/music/avatar.png";
    thumb.alt = ""; thumb.width = thumb.height = 48; thumb.loading = "lazy";
    const meta = el("div", undefined, "track-meta");
    meta.append(el("span", track.artist));
    text.append(el("strong", track.title), meta);
    identity.append(thumb, text); cell.append(identity);
    row.append(cell, el("td", selected.name, "track-album"), el("td", duration(track.duration_seconds), "track-duration"));
    body.append(row);
  });
  table.append(caption, head, body);
  const credits = el("details", undefined, "album-credits");
  credits.append(el("summary", "Credits & licences"));
  const sources = el("ul");
  for (const track of selected.tracks) {
    const item = el("li"), link = el("a", `${track.title} — ${track.artist}`);
    if (track.source?.startsWith("https://")) { link.href = track.source; link.target = "_blank"; link.rel = "noopener noreferrer"; }
    item.append(link, el("span", ` · ${track.license}`)); sources.append(item);
  }
  credits.append(sources);
  editor.append(table, credits);
}
function pageLink(target, label, cls) {
  const link = el("a", label, cls);
  const url = pageLocation(target);
  link.href = url.pathname + url.search;
  link.addEventListener("click", event => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    if (!saving) navigate(target);
  });
  return link;
}

function profileMiniature() {
  const id = current, seq = sequence;
  const link = pageLink("profile", undefined, "discord-profile overview-profile");
  link.setAttribute("aria-label", "Open Bot Profile");
  const header = el("div", undefined, "overview-profile-banner");
  const banner = el("img", undefined, "profile-banner");
  banner.alt = "";
  banner.hidden = true;
  banner.addEventListener("error", () => { banner.hidden = true; });
  header.append(banner);
  const frame = el("div", undefined, "profile-avatar-editor");
  const avatar = el("img", undefined, "profile-avatar");
  avatar.alt = "";
  avatar.src = "/music/avatar.png";
  avatar.addEventListener("error", () => { avatar.src = "/music/avatar.png"; }, { once: true });
  frame.append(avatar);
  const body = el("div", undefined, "profile-preview-body");
  const name = el("strong", "Bot Profile");
  const title = el("div", undefined, "profile-preview-name");
  title.append(name, el("span", "APP", "profile-app-badge"));
  const description = el("p", "", "profile-description");
  description.hidden = true;
  const status = el("small", "Loading server profile…");
  status.setAttribute("role", "status");
  body.append(el("span", "Bot profile", "overview-profile-label"), title, description, status);
  const overlay = el("span", "Open Bot Profile →", "overview-profile-overlay");
  overlay.setAttribute("aria-hidden", "true");
  link.append(header, frame, body, overlay);
  api(`/api/guilds/${id}/profile`).then(profile => {
    if (seq !== sequence || !link.isConnected) return;
    name.textContent = profile.nick || profile.default_name || "Vozen Music";
    avatar.src = profileImageUrl(profile, id, "avatar") || profile.default_avatar_url || "/music/avatar.png";
    const image = profileImageUrl(profile, id, "banner");
    if (image) {
      banner.src = image;
      banner.hidden = false;
    }
    description.textContent = profile.bio || "";
    description.hidden = !profile.bio;
    status.hidden = true;
  }).catch(() => {
    if (link.isConnected && seq === sequence) status.textContent = "Server profile could not be loaded.";
  });
  return link;
}

function renderOverview(editor) {
  const layout = el("div", undefined, "overview-layout");
  const choices = card("Your choices");
  choices.classList.add("overview-choices");
  const { session } = model;
  const station = id => catalog.find(item => item.id === id)?.name || (id === "my" ? "My Station" : id);
  const state = !session || session.active === false ? "Idle" : session.paused ? "Paused" : "Playing";
  const live = el("section", undefined, "overview-session");
  const playback = el("div", undefined, "overview-session-copy");
  playback.append(el("h2", state === "Idle" ? "Nothing playing" : session.track));
  if (state !== "Idle") playback.append(el("span", state, "overview-session-state"));
  const context = el("div", undefined, "overview-session-context");
  if (state === "Idle") {
    context.append(el("span", "Start in Discord"), el("code", "/play"));
  } else {
    context.append(el("span", station(session.station)), el("strong", `${session.volume}% volume`));
  }
  live.append(playback, context);
  live.dataset.state = state;

  const mode = { session: "Session", always: "24/7", auto_join: "Auto-join" }[draft.radio_mode];
  const skipAccess = draft.skip.access === "moderators" ? "Moderators only" : "Call members";
  const skipMode = draft.skip.mode === "majority" ? "Majority vote" : "One click";
  const summaries = {
    playback: `${station(draft.default_station)} · ${draft.volume}% · ${mode}`,
    access: `Skip · ${skipAccess} · ${skipMode}`,
    community: draft.statistics ? "Tracking allowed" : "Tracking off",
  };
  for (const [target, title] of Object.entries(settingsSections)) {
    const section = el("section", undefined, "overview-section");
    const copy = el("div");
    copy.append(el("h3", title), el("p", summaries[target]));
    const action = pageLink(target, "Edit →");
    action.setAttribute("aria-label", `Edit ${title}`);
    section.append(copy, action);
    choices.append(section);
  }
  const side = el("div", undefined, "overview-side");
  side.append(choices, profileMiniature());
  layout.append(stationShortcut(), side);
  editor.append(live, layout);
}

function stationShortcut() {
  const id = current;
  const stations = pageLink("stations", undefined, "overview-stations");
  stations.setAttribute("aria-label", "Explore stations");
  const covers = el("div", undefined, "overview-station-covers");
  for (const station of ["lofi", "rain", "night"]) {
    const cover = el("img");
    cover.src = `/music/albums/${station}-v1.webp`;
    cover.alt = "";
    covers.append(cover);
  }
  const body = el("div", undefined, "overview-stations-body");
  const heading = el("h2", "Stations");
  const details = el("div", undefined, "overview-track-details");
  const enabled = model.settings.statistics;
  details.append(el("p", enabled
    ? "Loading listening stats…"
    : "Browse instrumental stations. Enable time tracking in Community to see your server’s top track."));
  const footer = el("div", "Explore stations", "overview-stations-footer");
  const arrow = el("span", "→");
  arrow.setAttribute("aria-hidden", "true");
  footer.append(arrow);
  body.append(heading, details, footer);
  stations.append(covers, body);
  if (enabled) {
    api(`/api/guilds/${id}/track-listening`).then(({ enabled, top }) => {
      if (current !== id || !stations.isConnected) return;
      if (!top) {
        details.textContent = enabled
          ? "No listening stats yet. Play music in Discord with time tracking on."
          : "Time tracking is off for this server.";
        return;
      }
      if (top.cover) {
        const cover = el("img");
        cover.src = top.cover;
        cover.alt = "";
        covers.classList.add("is-featured");
        covers.replaceChildren(cover);
      }
      heading.textContent = "Most listened";
      details.replaceChildren(el("strong", top.title));
      if (top.artist) details.append(el("p", top.artist));
      const stats = el("dl", undefined, "overview-track-stats");
      for (const [label, seconds] of [["Track time", top.seconds], ["Server total", top.total_seconds]]) {
        const hours = Math.floor(seconds / 3600);
        const minutes = Math.floor(seconds / 60) % 60;
        const duration = hours ? `${hours} h ${minutes} min` : minutes ? `${minutes} min` : `${seconds} s`;
        const stat = el("div");
        stat.append(el("dt", label), el("dd", duration));
        stats.append(stat);
      }
      details.append(stats);
    }).catch(() => {
      if (current === id && stations.isConnected) details.textContent = "Listening stats could not load.";
    });
  }
  return stations;
}

function renderSettings(editor) {
  const nav = el("nav", undefined, "settings-jump");
  nav.setAttribute("aria-label", "Settings sections");
  const sections = {};
  for (const [key, title] of Object.entries(settingsSections)) {
    const heading = el("h2", title);
    heading.id = "settings-" + key;
    heading.tabIndex = -1;
    const section = el("section", undefined, "settings-section");
    section.dataset.settingsSection = key;
    section.setAttribute("aria-labelledby", heading.id);
    section.append(heading);
    const link = pageLink(key, title);
    link.dataset.settingsSection = key;
    nav.append(link);
    sections[key] = section;
  }
  const { playback, access, community } = sections;
  editor.append(nav, playback, access, community);
  showSettingsSection();

  playback.append(
    group(
      "Radio defaults",
      "Set the station, volume and when Music joins or leaves a call.",
      playbackFields(),
    ),
    group(
      "Player messages",
      "Choose what Music posts when playback starts.",
      playerMessageFields(),
    ),
  );
  const timer = card(
    "Sleep timer",
    "Stop playback and disconnect after a set number of minutes. Timer changes take effect immediately.",
    "h3",
  );
  timer.append(
    el(
      "p",
      model.session?.sleep_until
        ? `Current deadline: ${new Date(model.session.sleep_until * 1000).toLocaleString()}`
        : "No timer is set.",
    ),
  );
  const duration = el("input");
  duration.type = "number";
  duration.min = 1;
  duration.max = 720;
  duration.value = 30;
  duration.id = "timer-minutes";
  const label = el("label", "Stop after (minutes)");
  label.htmlFor = duration.id;
  timer.append(label, duration);
  const timerAction = async (cancel) => {
    if (!duration.validity.valid && !cancel) {
      duration.reportValidity();
      return;
    }
    const id = current;
    const result = await api(`/api/guilds/${id}/playback`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        minutes: cancel ? null : Number(duration.value),
        cancel,
      }),
    });
    if (id === current) {
      if (model.session) model.session.sleep_until = result.sleep_until;
      notice("Sleep timer updated.", "success");
      render();
    }
  };
  timer.append(
    button("Set timer", () => timerAction(false)),
    button("Cancel timer", () => timerAction(true)),
  );
  playback.append(timer);

  access.append(
    group(
      "Who can control the player",
      "Choose your DJs and decide when playback controls are restricted.",
      accessFields(),
    ),
    group(
      "Skip rules",
      "The same rules apply to the Skip button and /skip. DJs still need to meet the vote threshold.",
      skipFields(),
    ),
  );
  const channels = el("details", undefined, "settings-channel-rules");
  channels.append(el("summary", "Channel restrictions"));
  channels.append(el("p", "Choose where members can use Music. With no channels selected, a blocklist allows all channels and an allowlist allows none.", "help"));
  access.append(channels);
  for (const [key, label, voice] of [
    ["voice_rules", "Voice channels", true],
    ["text_rules", "Text channels", false],
  ])
    channels.append(
      group(
        label,
        "",
        [
          field(`${key}.allow`, "List behaviour", "select", [
            ["false", "Block selected channels"],
            ["true", "Allow only selected channels"],
          ]),
          field(
            `${key}.channels`,
            "Channels",
            "multi",
            channelOptions(voice).filter(([id]) => id),
          ),
        ],
      ),
    );

  community.append(
    group(
      "Server settings",
      "Choose whether members of this server can track their time with Music.",
      [
        field(
          "statistics",
          "Allow time tracking",
          "checkbox",
          [],
          "Members choose whether to take part. Use Save changes to apply this to the server.",
        ),
      ],
    ),
  );
  const personal = card(
    "Your activity & privacy",
    "Time counts while Music plays in your call and you aren’t deafened. It also contributes to server track totals without your name. Your choices apply immediately.",
    "h3",
  );
  const leaderboard = card(
    "Server leaderboard",
    "Only members who choose to appear are listed.",
    "h3",
  );
  const preferences = el("div");
  const ranking = el("div");
  const loading = el("p", "Loading your preferences…", "community-empty");
  loading.setAttribute("role", "status");
  preferences.append(loading);
  personal.append(preferences);
  leaderboard.append(ranking);
  const layout = el("div", undefined, "community-layout");
  layout.append(personal, leaderboard);
  community.append(layout);
  loadPersonal(current, sequence, preferences, ranking);
}

function render() {
  stopBrowserPreview();
  const editor = $("editor");
  editor.replaceChildren();
  $("music-pages").replaceChildren();
  for (const [key, label] of Object.entries(pages)) {
    const b = button(label, () => navigate(key));
    const namespace = "http://www.w3.org/2000/svg";
    const icon = document.createElementNS(namespace, "svg");
    icon.setAttribute("viewBox", "0 0 24 24");
    icon.setAttribute("aria-hidden", "true");
    const path = document.createElementNS(namespace, "path");
    path.setAttribute("d", pageIcons[key]);
    icon.append(path);
    const badge = el("span", undefined, "music-page-icon");
    badge.append(icon);
    b.prepend(badge);
    b.dataset.page = key;
    b.setAttribute("aria-current", key === page ? "page" : "false");
    $("music-pages").append(b);
  }
  const albumOpen = page === "stations" && [...catalog.map(s => s.id), ...proposals.map(s => s.id), "my"].includes(new URLSearchParams(location.search).get("album"));
  if (!albumOpen) editor.append(
    el("h1", pages[page] || pages.overview),
    el("p", descriptions[page] || descriptions.overview, "intro"),
  );
  if (page === "overview") renderOverview(editor);
  if (page === "settings") renderSettings(editor);
  if (page === "stations") {
    stationLibrary(editor);
    if (["night", "rain", "sleep"].some((id) => !catalog.some((s) => s.id === id)))
      editor.append(card("More stations", "Additional stations are being prepared."));
  }
  if (page === "profile") {
    renderProfile(editor);
    loadProfile(current, sequence);
  }
  const bar = el("div", undefined, "savebar");
  bar.id = "music-savebar";
  const status = el("p", "You have unsaved changes");
  status.id = "music-save-status";
  status.setAttribute("role", "status");
  const discard = button("Discard", () => {
    draft = JSON.parse(baseline);
    persistDraft();
    render();
  });
  discard.id = "music-discard";
  const saveButton = button("Save changes", save, true);
  saveButton.id = "music-save";
  bar.append(status, discard, saveButton);
  editor.append(bar);
  dependent();
  updateSave();
}
function normalizedProfileText(value) {
  const groups = { o: "0οоòóôõö", i: "1!іíìîï", e: "3еéèêë", a: "4@аàáâãä", s: "5$", t: "7τт", c: "çс", u: "ùúûü", n: "ñ", g: "ɡ" };
  return [...value.toLowerCase()].map(c => {
    const code = c.codePointAt(0);
    if (code >= 0xff01 && code <= 0xff5e) c = String.fromCodePoint(code - 0xfee0);
    for (const [letter, chars] of Object.entries(groups)) if (chars.includes(c)) return letter;
    return /^[a-z0-9]$/.test(c) ? c : "";
  }).join("");
}
function profileTextError(text, policy) {
  const compact = normalizedProfileText(text);
  const words = text.split(/[^\p{L}\p{N}]+/u).map(normalizedProfileText);
  return policy?.blocked_terms.some(term => compact.includes(term)) || policy?.blocked_words.some(term => words.includes(term) || compact === term)
    ? "Choose a different name. Discriminatory slurs and hate slogans are not allowed." : "";
}
function renderProfile(editor) {
  const c = card("Server profile", "Edit your profile below. Nothing changes in Discord until you apply.");
  c.classList.add("profile-editor-card");
  const form = el("form"), surface = el("div", undefined, "discord-profile");
  const view = { form, saved: {}, uploads: {}, pending: 0, loading: true, fields: {}, clear: {} };
  profileView = view;
  form.id = "profile-form";
  form.addEventListener("submit", e => { e.preventDefault(); saveProfile(false).catch(e => notice(e.message, "error")); });
  for (const [key, label, type, max] of [["nick", "Display name", "text", 32], ["avatar", "Avatar", "file"], ["banner", "Banner", "file"]]) {
    const wrap = el("div", undefined, "music-field"), input = el("input"), lab = el("label", label), help = el("small");
    input.name = key; input.id = `profile-${key}`; input.disabled = true;
    input.type = type;
    if (max) input.maxLength = max;
    if (type === "file") { input.accept = "image/png,image/jpeg,image/webp"; input.hidden = true; }
    lab.htmlFor = input.id;
    help.id = `profile-${key}-help`;
    input.setAttribute("aria-describedby", help.id);
    help.textContent = type === "file" ? "Static PNG, JPEG or WebP · up to 2 MB. Changes are previewed before upload." : "";
    wrap.append(lab, input, help);
    view.fields[key] = wrap;
    if (type === "file") {
      lab.className = "profile-sr-only";
      const clear = button(`Undo ${key} selection`, () => { input.value = ""; delete view.uploads[key]; input.setCustomValidity(""); notice(""); updateProfilePreview(view); });
      clear.className = "profile-image-undo";
      clear.setAttribute("aria-label", `Undo ${key} selection`); clear.title = `Undo ${key} selection`; clear.textContent = "↶";
      clear.hidden = true; view.clear[key] = clear;
      clear.disabled = true;
      const trigger = button("", () => input.click());
      trigger.disabled = true; trigger.className = "profile-image-trigger";
      trigger.setAttribute("aria-label", `Edit ${key}`);
      const pencil = el("span", "✎", "profile-pencil"); pencil.setAttribute("aria-hidden", "true");
      trigger.append(pencil, el("span", `Edit ${key}`, "profile-edit-label"));
      const area = el("div", undefined, `profile-${key}-editor`);
      const image = el("img", undefined, `profile-${key}`); image.alt = `${label} preview`;
      if (key === "avatar") image.src = "/music/avatar.png"; else image.hidden = true;
      view[key] = image;
      area.append(image, trigger, clear);
      wrap.prepend(area);
      input.addEventListener("change", async () => {
        notice("");
        delete view.uploads[key]; input.setCustomValidity("");
        const file = input.files[0];
        if (!file) { updateProfilePreview(view); return; }
        view.pending++;
        updateProfilePreview(view);
        try {
          const data = await readProfileFile(file);
          const image = new Image(); image.src = data; await image.decode();
          if (image.naturalWidth > 4096 || image.naturalHeight > 4096) throw Error("Images must be no larger than 4096 pixels on either side.");
          if (profileView === view && input.files[0] === file) view.uploads[key] = data;
        } catch (e) {
          if (profileView === view && input.files[0] === file) input.setCustomValidity(e.message || "Choose a valid image.");
        } finally { view.pending--; updateProfilePreview(view); }
      });
    } else input.addEventListener("input", () => { notice(""); updateProfilePreview(view); });
  }
  const body = el("div", undefined, "profile-preview-body");
  const name = el("div", undefined, "profile-preview-name");
  name.append(view.fields.nick, el("span", "APP", "profile-app-badge"));
  view.bio = el("p", "", "profile-description");
  view.bio.hidden = true;
  body.append(name, view.bio, el("p", "Server profile · Preview", "profile-server-label"));
  surface.append(view.fields.banner, view.fields.avatar, body);
  form.append(surface, el("p", "Select the banner or avatar to change it. Static PNG, JPEG or WebP, up to 2 MB. This preview approximates Discord.", "help"));
  view.apply = button("Apply profile", () => saveProfile(false), true); view.apply.disabled = true;
  const actions = el("div", undefined, "profile-actions");
  actions.append(view.apply, button("Restore Vozen profile", () => { if (confirm("Restore the default bot profile in this server?")) return saveProfile(true); }));
  form.append(el("p", "Applies to this server only. Discriminatory names are not allowed.", "help"), actions);
  view.error = el("p", "", "profile-error"); view.error.setAttribute("role", "status"); view.error.hidden = true;
  view.retry = button("Retry loading profile", () => loadProfile(current, sequence)); view.retry.hidden = true;
  form.append(view.error, view.retry);
  c.append(form); editor.append(c);
}
function updateProfilePreview(view) {
  if (profileView !== view || !view.form.isConnected) return;
  const nick = $("profile-nick");
  const message = profileTextError(nick.value, view.saved.policy) || (nick.value && (!nick.value.trim() || /[\u0000-\u001f\u007f]/.test(nick.value)) ? "Use a visible name without line breaks." : "");
  nick.setCustomValidity(message);
  nick.setAttribute("aria-invalid", String(!!message));
  $("profile-nick-help").textContent = `${[...nick.value].length}/${nick.maxLength} characters · Leave empty for the default name.`;
  nick.placeholder = view.saved.default_name || "Vozen Music";
  view.bio.textContent = view.saved.bio || "";
  view.bio.hidden = !view.saved.bio;
  const image = key => {
    if (view.uploads[key]) return view.uploads[key];
    return profileImageUrl(view.saved, current, key);
  };
  view.avatar.src = image("avatar") || view.saved.default_avatar_url || "/music/avatar.png";
  const banner = image("banner");
  view.banner.hidden = !banner;
  if (banner) view.banner.src = banner; else view.banner.removeAttribute("src");
  for (const key of ["avatar", "banner"]) view.clear[key].hidden = !$(`profile-${key}`).files.length;
  const invalid = [...view.form.querySelectorAll("input,textarea")].find(i => !i.validity.valid);
  view.error.textContent = invalid?.validationMessage || (view.pending ? "Preparing your image preview…" : "");
  view.error.hidden = !view.error.textContent;
  view.apply.disabled = view.loading || !!view.pending || !!view.saving || !!invalid;
}
function profileImageUrl(profile, guild, key) {
  const hash = profile[key], bot = profile.bot_id;
  return /^(a_)?[a-f0-9]{32}$/.test(hash || "") && /^\d+$/.test(bot || "") && /^\d+$/.test(guild)
    ? `https://cdn.discordapp.com/guilds/${guild}/users/${bot}/${key}s/${hash}.png?size=512` : "";
}
async function readProfileFile(file) {
  if (file.size > 2 * 1024 * 1024) throw Error("Images must be no larger than 2 MB.");
  if (!["image/png", "image/jpeg", "image/webp"].includes(file.type)) throw Error("Choose a PNG, JPEG or WebP image.");
  return new Promise((resolve, reject) => {
    const reader = new FileReader(); reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(Error("The image could not be read.")); reader.readAsDataURL(file);
  });
}
async function loadPersonal(id, seq, panel, leaderboard, focusId) {
  const connected = () => seq === sequence && current === id && panel.isConnected;
  try {
    const [me, ranking] = await Promise.all([
      api(`/api/guilds/${id}/personal`),
      api(`/api/guilds/${id}/community`),
    ]);
    if (!connected()) return;
    const enabled = me.privacy.enabled;
    const visible = enabled && me.privacy.visible;
    const serverEnabled = JSON.parse(baseline).statistics;
    let pending = false;
    const action = (label, payload, controlId = "") => {
      const control = button(label, async () => {
        if (pending || saving) return;
        if ((payload.erase_statistics || payload.clear_ratings) &&
            !confirm(`${label} in this server?`)) return;
        pending = true;
        control.disabled = true;
        try {
          await api(`/api/guilds/${id}/personal`, {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(payload),
          });
          if (connected()) {
            notice("Your preferences were saved.", "success");
            await loadPersonal(id, seq, panel, leaderboard, controlId);
          }
        } finally {
          pending = false;
          control.disabled = saving;
        }
      });
      if (controlId) control.id = controlId;
      control.disabled = saving;
      return control;
    };
    const tracking = el("div", undefined, "community-row");
    const trackingCopy = el("div");
    trackingCopy.append(
      el("h4", "My time tracking"),
      el("p", `${enabled ? (serverEnabled ? "On" : "Paused by server") : "Off"} · ${Math.floor(me.seconds / 60)} min recorded`),
    );
    tracking.append(trackingCopy, action(
      enabled ? "Turn off tracking" : "Turn on tracking",
      { privacy: { enabled: !enabled, visible: false } },
      "community-tracking",
    ));
    const visibility = el("div", undefined, "community-row");
    const visibilityCopy = el("div");
    visibilityCopy.append(
      el("h4", "My leaderboard visibility"),
      el("p", visible
        ? (serverEnabled ? "Visible to this server" : "Set to visible when the server leaderboard is available")
        : enabled ? "Hidden from the leaderboard" : "Turn on tracking to choose who can see your time."),
    );
    visibility.append(visibilityCopy);
    if (enabled) visibility.append(action(
      visible ? "Hide my entry" : "Show my entry",
      { privacy: { enabled: true, visible: !visible } },
      "community-visibility",
    ));
    const removal = el("details", undefined, "settings-data-removal");
    removal.append(el("summary", "Delete your data"));
    const removalActions = el("div", undefined, "settings-actions");
    removalActions.append(
      action("Erase my recorded time", { erase_statistics: true }),
      action("Delete my likes and dislikes", { clear_ratings: true }),
    );
    removal.append(el("p", "Erasing your time also turns off tracking. Your song ratings can be deleted separately."), removalActions);
    panel.replaceChildren(tracking, visibility, removal);
    if (focusId) $(focusId)?.focus({ preventScroll: true });

    const list = el("ol", undefined, "community-leaderboard");
    for (const [user, seconds] of ranking.rows) {
      const row = el("li");
      const entry = el("div");
      const member = el("span", user === viewerId ? "You" : "Member");
      if (user !== viewerId) member.append(el("small", `Discord ID ${user}`));
      entry.append(
        member,
        el("strong", `${Math.floor(seconds / 60)} min`),
      );
      row.append(entry);
      list.append(row);
    }
    leaderboard.replaceChildren(ranking.rows.length ? list : el("p",
      serverEnabled ? "No members have joined the leaderboard yet." : "Time tracking is turned off for this server.",
      "community-empty",
    ));
  } catch (e) {
    if (connected()) {
      const error = el("p", e.message);
      error.setAttribute("role", "status");
      panel.replaceChildren(error, button("Retry", () => loadPersonal(id, seq, panel, leaderboard)));
      leaderboard.replaceChildren(el("p", "The leaderboard could not load.", "community-empty"));
    }
  }
}
async function loadProfile(id, seq) {
  const view = profileView;
  try {
    const p = await api(`/api/guilds/${id}/profile`);
    if (seq !== sequence || page !== "profile" || view !== profileView) return false;
    view.saved = p;
    $("profile-nick").value = p.nick || "";
    for (const input of view.form.querySelectorAll("input,textarea,button")) input.disabled = !!view.saving;
    view.loading = false; view.retry.hidden = true;
    updateProfilePreview(view);
    return true;
  } catch (e) {
    if (id === current && view === profileView) { notice(e.message, "error"); view.retry.hidden = false; }
    return false;
  }
}
async function saveProfile(reset) {
  const view = profileView;
  if (view.saving) return;
  if (!reset && (view.loading || view.pending || !view.form.reportValidity())) return;
  const id = current,
    body = { reset };
  if (!reset) {
    body.nick = $("profile-nick").value;
    for (const key of ["avatar", "banner"]) {
      const file = $(`profile-${key}`).files[0];
      if (file) {
        body[key] = view.uploads[key] || await readProfileFile(file);
      }
    }
  }
  const buttons = [...view.form.querySelectorAll("input,textarea,button")];
  view.saving = true;
  buttons.forEach((b) => (b.disabled = true));
  try {
    await api(`/api/guilds/${id}/profile`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    if (id === current && profileView === view) {
      notice("Discord applied the profile to this server.", "success");
      if (await loadProfile(id, sequence)) {
        view.uploads = {};
        for (const key of ["avatar", "banner"]) $(`profile-${key}`).value = "";
      }
    }
  } finally {
    view.saving = false;
    buttons.forEach((b) => (b.disabled = view.loading));
    updateProfilePreview(view);
  }
}
function serverAvatar(server, className) {
  const avatar = el("span", undefined, className);
  avatar.setAttribute("aria-hidden", "true");
  avatar.dataset.initials = server.name.slice(0, 1).toUpperCase();
  if (/^\d+$/.test(server.id) && /^(a_)?[a-f0-9]{32}$/.test(server.icon || "")) {
    const image = el("img");
    image.alt = "";
    image.src = `https://cdn.discordapp.com/icons/${server.id}/${server.icon}.png?size=64`;
    image.addEventListener("error", () => image.remove());
    avatar.append(image);
  }
  return avatar;
}
function renderServers() {
  const select = $("server-select");
  $("selected-server-icon")?.remove();
  select.replaceChildren();
  for (const s of serverList) {
    const option = new Option(s.name, s.id, false, s.id === current);
    option.prepend(serverAvatar(s, "server-option-icon"));
    select.add(option);
    if (s.id === current) {
      const icon = serverAvatar(s, "server-option-icon");
      icon.id = "selected-server-icon";
      select.before(icon);
    }
  }
  select.add(new Option("Choose another server", "__choose_server__"));
  select.disabled = saving;
}
function showServerPicker() {
  if (saving) return;
  if (current && !persistDraft() && changed()) return;
  stopBrowserPreview();
  sequence++;
  current = "";
  draft = null;
  baseline = "";
  closeSections(false);
  for (const id of ["editor", "sidebar", "loading", "workspace-loading", "empty"])
    $(id).hidden = true;
  notice("");
  const url = new URL(location.href);
  url.pathname = "/music";
  url.searchParams.delete("server");
  url.searchParams.delete("page");
  url.searchParams.delete("section");
  url.searchParams.delete("album");
  history.replaceState(null, "", url);
  const choices = $("server-choices");
  choices.replaceChildren();
  for (const server of serverList) {
    const row = button("", () => {
      page = "overview";
      return loadServer(server.id);
    });
    row.className = "server-choice vp-server";
    const avatar = serverAvatar(server, "server-avatar vp-avatar");
    avatar.prepend(avatar.dataset.initials);
    row.append(avatar, el("span", server.name, "vp-name"));
    const arrow = el("span", "›", "server-arrow vp-arrow");
    arrow.setAttribute("aria-hidden", "true");
    row.append(arrow);
    choices.append(row);
  }
  $("picker-invite").href = $("sidebar").querySelector('a[href*="discord.com/oauth2"]').href;
  $("server-picker").hidden = false;
  $("main").scrollTop = 0;
  $("main").focus();
}
async function loadServer(id) {
  const seq = ++sequence;
  current = id;
  notice("");
  baseline = "";
  draft = null;
  $("server-picker").hidden = true;
  $("sidebar").hidden = false;
  renderServers();
  $("editor").hidden = true;
  $("empty").hidden = true;
  showLoading(true);
  try {
    const [data, stations] = await Promise.all([
      api(`/api/guilds/${id}/server-settings`),
      api("/api/stations"),
    ]);
    if (seq !== sequence) return;
    model = data;
    catalog = stations.stations;
    proposals = stations.proposals || [];
    baseline = JSON.stringify(data.settings);
    draft = structuredClone(data.settings);
    try {
      const saved = JSON.parse(
        localStorage.getItem(`music-draft:${id}`) || "null",
      );
      if (
        saved &&
        saved.revision === draft.revision &&
        validDraft(saved, data.settings)
      ) {
        draft = saved;
        notice("Your unsaved draft was restored.");
      } else if (saved)
        notice(
          "An older browser draft exists. Server settings changed; it was not applied.",
        );
    } catch {
      notice("This browser could not restore your draft.");
    }
    const legacySection = Object.hasOwn(settingsSections, page) ? page : null;
    if (legacySection) page = "settings";
    if (!Object.hasOwn(pages, page)) page = "overview";
    $("editor").hidden = false;
    const url = new URL(location.href);
    url.pathname = `/music/${id}`;
    url.searchParams.delete("server");
    if (page === "overview") url.searchParams.delete("page");
    else if (url.searchParams.has("page")) url.searchParams.set("page", page);
    if (legacySection) url.searchParams.set("section", legacySection);
    history.replaceState(null, "", url);
    render();
    if (page === "settings") focusSettingsSection();
  } catch (e) {
    if (seq === sequence) notice(e.message, "error", e.status === 401 ? null : () => loadServer(id));
  } finally {
    if (seq === sequence) showLoading(null);
  }
}
async function refreshServers() {
  notice("");
  showLoading(!!requestedServer());
  try {
    const data = await api("/api/guilds");
    serverList = data.guilds;
    renderServers();
    if (!serverList.length) {
      showLoading(null);
      if (!$("empty").querySelector(".install"))
        $("empty").append($("sidebar").querySelector('a[href*="discord.com/oauth2"]').cloneNode(true));
      $("empty").hidden = false;
      return;
    }
    const wanted = requestedServer();
    if (serverList.some((s) => s.id === wanted)) await loadServer(wanted);
    else showServerPicker();
  } catch (e) {
    showLoading(null);
    notice(e.message, "error", e.status === 401 ? null : refreshServers);
  } finally {
    $("loading").hidden = true;
  }
}
function closeSections(focus) {
  $("sidebar").classList.remove("sections-open");
  $("sections-trigger").setAttribute("aria-expanded", "false");
  if (focus) $("sections-trigger").focus();
}
$("server-select").addEventListener("change", (e) => {
  if (saving) return;
  if (e.target.value === "__choose_server__") {
    e.target.value = current;
    showServerPicker();
    return;
  }
  loadServer(e.target.value);
});
$("product-switcher").addEventListener("change", (e) => {
  if (
    saving ||
    (changed() && !confirm("Your draft is kept in this browser. Leave Music?"))
  ) {
    e.target.value = "music";
    return;
  }
  location.assign(
    e.target.value === "tts"
      ? "https://vozen.org/dashboard.html"
      : "https://vozen.org/panel/features/",
  );
});
$("sections-trigger").addEventListener("click", () => {
  $("sections-trigger").setAttribute(
    "aria-expanded",
    String($("sidebar").classList.toggle("sections-open")),
  );
});
$("sidebar").addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    closeSections(true);
  }
});
$("refresh").addEventListener("click", refreshServers);
$("logout").addEventListener("click", async () => {
  try {
    await api("/auth/logout", { method: "POST" });
    signedOut();
  } catch (e) {
    notice(e.message, "error");
  }
});
window.addEventListener("beforeunload", (e) => {
  if (changed()) {
    e.preventDefault();
    e.returnValue = "";
  }
});
window.addEventListener("pagehide", stopBrowserPreview);
window.addEventListener("popstate", () => {
  if (!draft || requestedServer() !== current) return;
  page = new URLSearchParams(location.search).get("page") || "overview";
  if (Object.hasOwn(settingsSections, page)) {
    history.replaceState(null, "", pageLocation(page));
    page = "settings";
  }
  if (!Object.hasOwn(pages, page)) page = "overview";
  render();
  if (!focusSettingsSection()) {
    $("main").scrollTop = 0;
    $("main").focus();
  }
});
if (window.visualViewport) {
  const update = () =>
    document.documentElement.style.setProperty(
      "--keyboard-offset",
      Math.max(
        0,
        innerHeight - visualViewport.height - visualViewport.offsetTop,
      ) + "px",
    );
  visualViewport.addEventListener("resize", update);
}
async function loadDashboard() {
  notice("");
  showLoading(!!requestedServer());
  try {
    const me = await api("/api/me");
    viewerId = me.user.id;
    csrf = me.csrf;
    renderAccount(me.user);
    await refreshServers();
  } catch (e) {
    if (e.status === 401) {
      location.replace("https://vozen.org/account/?music=1");
      return;
    }
    showLoading(null);
    notice(e.message, "error", loadDashboard);
  }
}
loadDashboard();
