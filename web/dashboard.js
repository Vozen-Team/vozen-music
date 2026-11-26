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
  acceptedUrl = location.href,
  serverList = [],
  model = null,
  baseline = "",
  draft = null,
  catalog = [],
  proposals = [],
  page = new URLSearchParams(location.search).get("page") || "overview";
const pages = {
  overview: t("Overview"),
  settings: t("Settings"),
  stations: t("Stations"),
  profile: t("Bot Profile"),
};
const pageIcons = {
  overview: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
  settings: "M4 7h16 M4 17h16 M8 4v6 M16 14v6",
  stations: "M3 10v4 M7 7v10 M12 4v16 M17 7v10 M21 10v4",
  profile: "M16 8a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z M4 21v-2a4 4 0 0 1 4-4h8a4 4 0 0 1 4 4v2",
};
const settingsSections = {
  playback: t("Playback"),
  access: t("Access & Channels"),
  community: t("Community"),
};
function el(tag, text, cls) {
  const e = document.createElement(tag);
  if (text !== undefined) bindMusicText(e, text);
  if (cls) e.className = cls;
  return e;
}
function notice(message, kind = "info", retry = null) {
  bindMusicText($("notice"), typeof message === "string" && message ? t(message) : message);
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
    // Logout and other bodiless successes answer 204 No Content.
    if (r.status === 204) return null;
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
  const loader = $("workspace-loading");
  if (workspace === true) {
    const album = page === "stations" && new URLSearchParams(location.search).get("album");
    loader.dataset.view = album ? "album" : Object.hasOwn(settingsSections, page) ? "settings" : Object.hasOwn(pages, page) ? page : "overview";
  }
  loader.hidden = workspace !== true;
}
function signedOut() {
  stopBrowserPreview();
  csrf = "";
  viewerId = "";
  current = "";
  draft = null;
  baseline = "";
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
    // Channel ids saved as digit strings may be cleared to null in a draft.
    if (typeof v === "string" && /^\d+$/.test(v))
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
function draftKey(id) {
  return `music-draft:${viewerId}:${id}`;
}
function persistDraft() {
  try {
    if (changed())
      localStorage.setItem(draftKey(current), JSON.stringify(draft));
    else localStorage.removeItem(draftKey(current));
    return true;
  } catch {
    notice(
      t("This browser could not preserve your draft. Keep this tab open until you save."),
      "error",
    );
    return false;
  }
}
function canLeaveServer() {
  return !saving && (!changed() || persistDraft());
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
// Server settings the API refuses to change without Vozen Premium on the server.
const premiumFields = ["volume", "default_station", "voice_channel", "dj_roles", "djs_only",
  "player_locked", "announcements", "announcement_channel"];
const premiumPlansUrl = "https://vozen.org/premium#plans";
// "unrestricted" means plan checks are off: every feature is available and no plan is shown.
function musicPlan() {
  return model?.music_access?.plan || "unrestricted";
}
function plansApply() {
  return musicPlan() !== "unrestricted";
}
function sharedMusicControlsLocked() {
  return plansApply() && model.music_access.shared === false;
}
function premiumField(path) {
  return premiumFields.includes(path) || /^(voice_rules|text_rules)\./.test(path);
}
function planPill(tier, text, locked = false) {
  const pill = el("span", text, "music-plan-pill");
  pill.dataset.tier = tier;
  if (locked) pill.dataset.locked = "";
  return pill;
}
function premiumLink(text) {
  const link = el("a", text, "music-plan-link");
  link.href = premiumPlansUrl;
  link.target = "_blank";
  link.rel = "noopener";
  return link;
}
// What Music actually does with these settings while they are locked.
const lockedBehaviour = {
  volume: t("Without Premium, Music plays at 100%."),
  default_station: t("Without Premium, /play continues the last station."),
};
function field(path, label, type = "text", options = [], help = "") {
  const wrap = el("div", undefined, "music-field"),
    id = `field-${path.replaceAll(".", "-")}`;
  const l = el("label", label);
  l.htmlFor = id;
  const copy = el("div", undefined, "settings-field-copy");
  const described = [];
  if (plansApply() && premiumField(path)) {
    const locked = sharedMusicControlsLocked();
    const head = el("div", undefined, "settings-field-head");
    const pill = planPill("premium", locked ? t("Premium required") : t("Premium"), locked);
    pill.id = `${id}-plan`;
    described.push(pill.id);
    head.append(l, pill);
    copy.append(head);
    if (locked) {
      wrap.dataset.musicPremiumLocked = "";
      if (lockedBehaviour[path]) {
        const note = el("small", lockedBehaviour[path], "music-plan-note");
        note.id = `${id}-locked`;
        described.push(note.id);
        copy.append(note);
      }
    }
  } else copy.append(l);
  let input;
  if (type === "select") {
    input = el("select");
    for (const [value, text, disabled] of options) {
      const option = new Option("", value);
      bindMusicText(option, text);
      option.disabled = !!disabled;
      input.add(option);
    }
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
    if (!options.length) input.append(el("p", t("No resources available.")));
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
        el("span", t("Unavailable resource ({id}) — uncheck to remove", { id: value })),
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
        bindMusicText(input, "", "validity");
        if (
          type === "number" &&
          (!input.validity.valid || input.value === "")
        ) {
          bindMusicText(input, t("Choose a whole number from 0 to 100."), "validity");
          return;
        }
        bindMusicText(input, "", "validity");
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
    described.unshift(hint.id);
    // Keep the field's own description above any plan note.
    copy.insertBefore(hint, copy.querySelector(".music-plan-note"));
  }
  if (described.length) input.setAttribute("aria-describedby", described.join(" "));
  wrap.append(copy, input);
  return wrap;
}
function stationOptions() {
  return catalog.map((s) => [s.id, s.name]);
}
function channelOptions(voice) {
  return [
    ["", t("None")],
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
    field("default_station", t("Default station"), "select", stationOptions(),
      t("Used when someone starts Music without picking a station.")),
    field(
      "volume",
      t("Default and current volume"),
      "number",
      [],
      t("Set the volume for the current player and future sessions, from 0 to 100."),
    ),
    field("radio_mode", t("Radio mode"), "select", [
      ["session", t("Session · Leave when empty")],
      ["always", t("24/7 · Stay connected")],
      ["auto_join", t("Auto-join · Start on entry"),
        sharedMusicControlsLocked() && get("radio_mode") !== "auto_join"],
    ], radioModeHelp()),
    field(
      "voice_channel",
      t("Fixed voice channel"),
      "select",
      channelOptions(true),
      t("Limit playback to one voice channel. Required for Auto-join."),
    ),
  ];
}

function playerMessageFields() {
  return [
    field("announcements", t("Show the player automatically"), "checkbox", [],
      t("Post the player in the voice channel’s chat.")),
    field(
      "announcement_channel",
      t("Auto-join notification channel"),
      "select",
      channelOptions(false),
      t("Choose where Auto-join posts its notification. Required in Auto-join mode."),
    ),
    field("tips", t("Show command tips on the player"), "checkbox"),
  ];
}
function accessFields() {
  return [
    field(
      "djs_only",
      t("DJ-only controls"),
      "checkbox",
      [],
      t("Only DJs and server managers can control playback."),
    ),
    field("player_locked", t("Lock the player"), "checkbox", [],
      t("Temporarily restrict playback controls to DJs and server managers.")),
    field("dj_roles", t("DJ roles"), "multi", roles(), t("Members with any of these roles are DJs.")),
  ];
}

function skipFields() {
  return [
    field("skip.access", t("Who can skip?"), "select", [
      ["everyone", t("Everyone in the call")],
      ["moderators", t("Moderators only")],
    ]),
    field("skip.mode", t("How does a skip happen?"), "select", [
      ["instant", t("One click")],
      ["majority", t("Majority vote")],
    ], t("Choose whether one person can skip or listeners need to vote.")),
    conditional(
      field(
        "skip.majority_scope",
        t("Who counts towards the majority?"),
        "select",
        [
          ["call", t("Everyone in the call")],
          ["eligible", t("Only eligible voters")],
        ],
      ),
      "skip.mode=majority",
    ),
    conditional(
      field(
        "skip.moderator_roles",
        t("Moderator roles"),
        "multi",
        roles(),
        t("Leave empty to use moderation permissions. Managers still need to be in the call and satisfy the vote threshold."),
      ),
      "skip.access=moderators",
    ),
  ];
}
function updateSave() {
  for (const c of document.querySelectorAll(
    "#editor input,#editor select,#editor button",
  ))
    c.disabled = saving || !!c.closest("[data-music-premium-locked]");
  for (const control of $("music-pages").querySelectorAll("button"))
    control.disabled = saving || !draft;
  const bar = $("music-savebar");
  if (!bar) return;
  bar.hidden = !changed() && !saving;
  $("music-save").disabled = saving || !changed();
  $("music-discard").disabled = saving;
  bindMusicText($("music-save-status"), t(saving ? "Saving…" : "You have unsaved changes"));
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
  const id = current, seq = sequence;
  saving = true;
  updateSave();
  try {
    const result = await api(`/api/guilds/${id}/server-settings`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(draft),
    });
    if (seq !== sequence) return;
    draft = structuredClone(result.settings);
    baseline = JSON.stringify(draft);
    model.settings = structuredClone(draft);
    persistDraft();
    notice(
      result.warning
        ? t("Settings saved for this server. {warning}", { warning: t(result.warning) })
        : t("Settings saved for this server."),
      result.warning ? "info" : "success",
    );
    // Rebuilding the Bot Profile page would drop its unapplied name and images.
    if (page === "profile") updateSave();
    else render();
  } catch (e) {
    if (e.status === 409 && seq === sequence) await rebaseDraft(id, seq);
    else notice(e.message, "error");
  } finally {
    saving = false;
    updateSave();
  }
}
// Another manager saved first: carry this user's edited fields onto the latest
// settings, so nothing they or the other manager saved is silently lost.
async function rebaseDraft(id, seq) {
  const before = JSON.parse(baseline);
  const mine = draft;
  let latest;
  try {
    latest = await api(`/api/guilds/${id}/server-settings`);
  } catch (e) {
    notice(e.message, "error");
    return;
  }
  if (seq !== sequence) return;
  const next = structuredClone(latest.settings);
  for (const key of Object.keys(next)) {
    if (key !== "revision" && JSON.stringify(mine[key]) !== JSON.stringify(before[key]))
      next[key] = structuredClone(mine[key]);
  }
  model = latest;
  model.settings = structuredClone(latest.settings);
  baseline = JSON.stringify(latest.settings);
  draft = next;
  persistDraft();
  render();
  notice(t("Someone else saved these settings. Your changes were applied to the latest version; review them and save again."), "info");
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
function acceptLocation(url, push = false) {
  const destination = new URL(url, location.href);
  if (destination.searchParams.has("lang") || new URL(location.href).searchParams.has("lang"))
    destination.searchParams.set("lang", musicLanguage);
  if (push) history.pushState(null, "", destination);
  else history.replaceState(null, "", destination);
  acceptedUrl = location.href;
  refreshMusicLanguageLinks();
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
  if (!draft || saving) return;
  const url = pageLocation(next);
  const previous = page;
  page = url.searchParams.get("page") || "overview";
  acceptLocation(url);
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
    id: "my", name: t("My Station"), cover: "/music/albums/my-v1.webp", tracks: [],
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
    if (!draft || saving) return;
    acceptLocation(event.currentTarget.href, true);
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
    const label = el("span", t("Personalized radio"), "station-featured-label");
    if (plansApply()) {
      const locked = sharedMusicControlsLocked();
      label.append(" ", planPill("premium", locked ? t("Premium required") : t("Premium"), locked));
    }
    intro.append(label, el("h2", personal.name), el("span", t("A mix shaped by the people in your call.")));
    const arrow = el("span", "→", "station-featured-arrow");
    arrow.setAttribute("aria-hidden", "true");
    featured.append(cover, intro, arrow);
    library.append(featured);

    const groups = [
      [t("Slow down"), ["lofi", "ambient", "night", "rain", "sleep"]],
      [t("Jazz & acoustic"), ["jazz", "piano", "acoustic", "bossa-nova"]],
      [t("Electronic"), ["electronic", "synthwave", "chill-house", "chiptune"]],
      [t("Rock, scores & bass"), ["rock", "cinematic", "drum-and-bass"]],
    ];
    const remaining = new Map(stations.filter(s => s.id !== "my").map(s => [s.id, s]));
    const addShelf = (title, entries) => {
      if (!entries.length) return;
      const shelf = el("section", undefined, "station-shelf");
      const grid = el("div", undefined, "station-shelf-grid");
      for (const station of entries) {
        const album = albumLink(station);
        album.append(art(station), el("strong", station.name), el("span", station.proposed ? t("In preparation") : t("{count} tracks", { count: station.tracks.length })));
        grid.append(album);
      }
      shelf.append(el("h2", title), grid);
      library.append(shelf);
    };
    for (const [title, ids] of groups) {
      addShelf(title, ids.map(id => remaining.get(id)).filter(Boolean));
      ids.forEach(id => remaining.delete(id));
    }
    addShelf(t("More stations"), [...remaining.values()]);
    editor.append(library);
    return;
  }
  const back = el("a", undefined, "button quiet album-back");
  const arrow = el("span", undefined, "album-back-icon");
  arrow.setAttribute("aria-hidden", "true");
  back.append(arrow, el("span", t("All stations")));
  back.href = linkTo(null);
  back.addEventListener("click", openAlbum);
  const hero = el("section", undefined, "station-album-header");
  const info = el("div");
  const metadata = el("p", undefined, "album-metadata");
  if (selected.proposed) bindMusicText(metadata, t("In preparation · Not available for playback"));
  else if (selected.id === "my") bindMusicText(metadata, t("Built from the listeners in your call."));
  else metadata.append(el("strong", t("{count} tracks", { count: selected.tracks.length })), el("span", t("Various artists")));
  info.append(el("p", selected.id === "my" ? t("Personalized radio") : t("Instrumental radio"), "album-label"), el("h1", selected.name), metadata);
  const cover = art(selected);
  cover.loading = "eager";
  hero.append(cover, back, info);
  editor.append(hero);
  if (selected.proposed) {
    editor.append(card(selected.name, t(selected.description)));
    editor.append(card(t("Music selection in progress"), t("This station will open once its instrumental tracks have passed the listening and license checks.")));
    return;
  }
  if (selected.id === "my") {
    editor.append(card(t("A station that follows your listeners"), t("Four favorite selections for every discovery selection, using the people currently in the call. Each person has equal weight; dislikes exclude recordings. Use /like, /dislike and /library. My Station has no fixed tracklist.")));
    editor.append(card(t("Only from Music’s approved catalog"), t("My Station chooses from the instrumental recordings in the stations above. Like tracks to shape future selections, or dislike them to exclude them while you are in the call. Manage your ratings with /library. Uploads, external links and tracks outside the catalog are not accepted.")));
    return;
  }
  const duration = seconds => Number.isFinite(seconds) && seconds >= 0
    ? `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}` : "—";
  const audio = el("audio");
  audio.preload = "none"; audio.volume = 0.5; audio.hidden = true;
  browserPreview = audio;
  const previewStatus = el("p", t("Preview tracks in this browser. Discord playback stays unchanged."), "browser-preview-status");
  previewStatus.setAttribute("role", "status");
  let active = null, request = 0;
  const controls = new Map();
  const syncPreview = () => {
    for (const [id, { button: control, row, track }] of controls) {
      const playing = active?.id === id && !audio.paused && !audio.ended;
      control.classList.toggle("is-playing", playing);
      row.classList.toggle("is-previewing", playing);
      control.setAttribute("aria-pressed", String(playing));
      bindMusicText(control, t(playing ? "Pause preview of {title}" : "Play preview of {title}", { title: track.title }), "aria-label");
    }
    bindMusicText(previewStatus, active ? t("{status}: {title} — {artist}. Only in this browser.", { status: t(audio.ended ? "Finished" : audio.paused ? "Paused" : "Previewing"), title: active.title, artist: active.artist }) : t("Preview tracks in this browser. Discord playback stays unchanged."));
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
        notice(t("This track could not be previewed. Reload or try another track."), "error");
      }
    }
    syncPreview();
  };
  editor.append(audio, previewStatus);
  const table = el("table", undefined, "station-tracks");
  const caption = el("caption", t("{station} — tracks and credits", { station: selected.name }), "station-sr-only");
  const head = el("thead"), headings = el("tr"), body = el("tbody");
  for (const [label, cls] of [["#", "track-number"], [t("Title"), ""], [t("Album"), "track-album"], [t("Duration"), "track-duration"]]) {
    const th = el("th", label, cls);
    th.scope = "col"; headings.append(th);
  }
  head.append(headings);
  selected.tracks.forEach((track, index) => {
    const row = el("tr");
    const position = el("td", undefined, "track-number");
    const play = button("", () => playPreview(track));
    play.className = "track-preview";
    bindMusicText(play, t("Play preview of {title}", { title: track.title }), "aria-label");
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
  credits.append(el("summary", t("Credits & licences")));
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
  bindMusicText(link, t("Open Bot Profile"), "aria-label");
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
  const name = el("strong", t("Bot Profile"));
  const title = el("div", undefined, "profile-preview-name");
  title.append(name, el("span", "APP", "profile-app-badge"));
  const description = el("p", "", "profile-description");
  description.hidden = true;
  const status = el("small", t("Loading server profile…"));
  status.setAttribute("role", "status");
  body.append(el("span", t("Bot profile"), "overview-profile-label"), title, description, status);
  const overlay = el("span", t("Open Bot Profile →"), "overview-profile-overlay");
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
    if (link.isConnected && seq === sequence) bindMusicText(status, t("Server profile could not be loaded."));
  });
  return link;
}

// The plan is resolved for this viewer on this server: "plus" is the viewer's own
// plan, "premium" is the server's, and "free" means neither applies here.
function planSummary() {
  const plan = musicPlan();
  const copy = {
    premium: [t("Premium server"), t("Vozen Premium is active here"),
      t("Everyone in this server gets My Station, call volume and the settings marked Premium.")],
    plus: [t("Vozen Plus"), t("You have Vozen Plus"),
      t("Skipping, better audio while you’re in the call and unlimited station changes come with you. Settings marked Premium need Vozen Premium on this server.")],
    free: [t("Free"), t("Vozen Premium isn’t active here"),
      t("All stations and the shared player are free. Vozen Plus adds skipping and better audio for you; Vozen Premium unlocks the settings marked Premium for everyone here.")],
    unknown: [t("Plan unavailable"), t("Music couldn’t check the plan"), planUnknownText],
  }[plan] || [];
  const [badge, title, text] = copy;
  const c = el("section", undefined, "music-card overview-plan");
  c.dataset.plan = plan;
  const head = el("div", undefined, "overview-plan-head");
  head.append(el("h2", title), planPill(plan === "premium" ? "premium" : plan === "plus" ? "plus" : "free", badge));
  c.append(head, el("p", text, "help"));
  if (plan !== "premium" && plan !== "unknown")
    c.append(premiumLink(plan === "plus" ? t("View Premium") : t("Compare plans")));
  return c;
}
// Auto-join is a Premium setting; the Top.gg vote only gates 24/7.
function radioModeHelp() {
  if (!sharedMusicControlsLocked()) return "";
  return model.music_access?.radio_vote
    ? t("Auto-join needs Vozen Premium on this server. Switching to 24/7 needs a current Top.gg vote for Vozen Music; Vozen Plus doesn’t need one.")
    : t("Auto-join needs Vozen Premium on this server.");
}
// Shown while the plan resolver does not answer: Premium settings stay locked.
const planUnknownText = t("Premium settings stay locked until Music can check this server’s plan again. Other settings still save, and saved choices still apply.");
// One line above locked settings, so each locked field only needs its badge.
function premiumCallout(text) {
  const c = el("div", undefined, "music-plan-callout");
  c.append(el("p", text), premiumLink(t("View Premium")));
  return c;
}

function renderOverview(editor) {
  const layout = el("div", undefined, "overview-layout");
  const choices = card(t("Your choices"));
  choices.classList.add("overview-choices");
  const { session } = model;
  const station = id => catalog.find(item => item.id === id)?.name || (id === "my" ? t("My Station") : id);
  const state = !session || session.active === false ? "Idle" : session.paused ? "Paused" : "Playing";
  const live = el("section", undefined, "overview-session");
  const playback = el("div", undefined, "overview-session-copy");
  playback.append(el("h2", state === "Idle" ? t("Nothing playing") : session.track));
  if (state !== "Idle") playback.append(el("span", t(state), "overview-session-state"));
  const context = el("div", undefined, "overview-session-context");
  if (state === "Idle") {
    context.append(el("span", t("Start in Discord")), el("code", "/play"));
  } else {
    context.append(el("span", station(session.station)), el("strong", t("{volume}% volume", { volume: session.volume })));
  }
  live.append(playback, context);
  live.dataset.state = state;

  const mode = { session: t("Session"), always: "24/7", auto_join: t("Auto-join") }[draft.radio_mode];
  const skipAccess = draft.skip.access === "moderators" ? t("Moderators only") : t("Call members");
  const skipMode = draft.skip.mode === "majority" ? t("Majority vote") : t("One click");
  const summaries = {
    // Locked volume is not applied: Music plays at 100% without Premium.
    playback: t("{station} · {volume}% · {mode}", { station: station(draft.default_station), volume: sharedMusicControlsLocked() ? 100 : draft.volume, mode }),
    access: t("Skip · {access} · {mode}", { access: skipAccess, mode: skipMode }),
    community: draft.statistics ? t("Tracking allowed") : t("Tracking off"),
  };
  for (const [target, title] of Object.entries(settingsSections)) {
    const section = el("section", undefined, "overview-section");
    const copy = el("div");
    copy.append(el("h3", title), el("p", summaries[target]));
    const action = pageLink(target, t("Edit →"));
    bindMusicText(action, t("Edit {section}", { section: title }), "aria-label");
    section.append(copy, action);
    choices.append(section);
  }
  const side = el("div", undefined, "overview-side");
  if (plansApply()) side.append(planSummary());
  side.append(choices, profileMiniature());
  layout.append(stationShortcut(), side);
  editor.append(live, layout);
}

function stationShortcut() {
  const id = current;
  const stations = pageLink("stations", undefined, "overview-stations");
  bindMusicText(stations, t("Explore stations"), "aria-label");
  const covers = el("div", undefined, "overview-station-covers");
  for (const station of ["lofi", "rain", "night"]) {
    const cover = el("img");
    cover.src = `/music/albums/${station}-v1.webp`;
    cover.alt = "";
    covers.append(cover);
  }
  const body = el("div", undefined, "overview-stations-body");
  const heading = el("h2", t("Stations"));
  const details = el("div", undefined, "overview-track-details");
  const enabled = model.settings.statistics;
  details.append(el("p", enabled
    ? t("Loading listening stats…")
    : t("Browse instrumental stations. Enable time tracking in Community to see your server’s top track.")));
  const footer = el("div", t("Explore stations"), "overview-stations-footer");
  const arrow = el("span", "→");
  arrow.setAttribute("aria-hidden", "true");
  footer.append(arrow);
  body.append(heading, details, footer);
  stations.append(covers, body);
  if (enabled) {
    api(`/api/guilds/${id}/track-listening`).then(({ enabled, top }) => {
      if (current !== id || !stations.isConnected) return;
      if (!top) {
        bindMusicText(details, enabled
          ? t("No listening stats yet. Play music in Discord with time tracking on.")
          : t("Time tracking is off for this server."));
        return;
      }
      if (top.cover) {
        const cover = el("img");
        cover.src = top.cover;
        cover.alt = "";
        covers.classList.add("is-featured");
        covers.replaceChildren(cover);
      }
      bindMusicText(heading, t("Most listened"));
      details.replaceChildren(el("strong", top.title));
      if (top.artist) details.append(el("p", top.artist));
      const stats = el("dl", undefined, "overview-track-stats");
      for (const [label, seconds] of [[t("Track time"), top.seconds], [t("Server total"), top.total_seconds]]) {
        const hours = Math.floor(seconds / 3600);
        const minutes = Math.floor(seconds / 60) % 60;
        const duration = hours ? t("{hours} h {minutes} min", { hours, minutes }) : minutes ? t("{minutes} min", { minutes }) : t("{seconds} s", { seconds });
        const stat = el("div");
        stat.append(el("dt", label), el("dd", duration));
        stats.append(stat);
      }
      details.append(stats);
    }).catch(() => {
      if (current === id && stations.isConnected) bindMusicText(details, t("Listening stats could not load."));
    });
  }
  return stations;
}

function renderSettings(editor) {
  const nav = el("nav", undefined, "settings-jump");
  bindMusicText(nav, t("Settings sections"), "aria-label");
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
  if (sharedMusicControlsLocked()) {
    const plan = musicPlan();
    const text = plan === "unknown"
      ? planUnknownText
      : plan === "plus"
        ? t("Your Vozen Plus perks are personal. Settings marked Premium required need Vozen Premium on this server; their saved choices still apply.")
        : t("Settings marked Premium required need Vozen Premium on this server. Their saved choices still apply.");
    playback.append(premiumCallout(text));
    access.append(premiumCallout(text));
  }

  playback.append(
    group(
      t("Radio defaults"),
      t("Set the station, volume and when Music joins or leaves a call."),
      playbackFields(),
    ),
    group(
      t("Player messages"),
      t("Choose what Music posts when playback starts."),
      playerMessageFields(),
    ),
  );
  const timer = card(
    t("Sleep timer"),
    t("Stop playback and disconnect after a set number of minutes. Timer changes take effect immediately."),
    "h3",
  );
  timer.append(
    el(
      "p",
      model.session?.sleep_until
        ? t("Current deadline: {deadline}", { deadline: new Date(model.session.sleep_until * 1000) })
        : t("No timer is set."),
    ),
  );
  const duration = el("input");
  duration.type = "number";
  duration.min = 1;
  duration.max = 720;
  duration.value = 30;
  duration.id = "timer-minutes";
  const label = el("label", t("Stop after (minutes)"));
  label.htmlFor = duration.id;
  timer.append(label, duration);
  const timerAction = async (cancel) => {
    if (!duration.validity.valid && !cancel) {
      duration.reportValidity();
      return;
    }
    const id = current, seq = sequence;
    const result = await api(`/api/guilds/${id}/playback`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        minutes: cancel ? null : Number(duration.value),
        cancel,
      }),
    });
    if (seq === sequence && id === current && draft) {
      if (model.session) model.session.sleep_until = result.sleep_until;
      notice(t("Sleep timer updated."), "success");
      render();
    }
  };
  timer.append(
    button(t("Set timer"), () => timerAction(false)),
    button(t("Cancel timer"), () => timerAction(true)),
  );
  playback.append(timer);

  access.append(
    group(
      t("Who can control the player"),
      t("Choose your DJs and decide when playback controls are restricted."),
      accessFields(),
    ),
    group(
      t("Skip rules"),
      t("The same rules apply to the Skip button and /skip. DJs still need to meet the vote threshold."),
      skipFields(),
    ),
  );
  const channels = el("details", undefined, "settings-channel-rules");
  channels.append(el("summary", t("Channel restrictions")));
  channels.append(el("p", t("Choose where members can use Music. With no channels selected, a blocklist allows all channels and an allowlist allows none."), "help"));
  access.append(channels);
  for (const [key, label, voice] of [
    ["voice_rules", t("Voice channels"), true],
    ["text_rules", t("Text channels"), false],
  ])
    channels.append(
      group(
        label,
        "",
        [
          field(`${key}.allow`, t("List behaviour"), "select", [
            ["false", t("Block selected channels")],
            ["true", t("Allow only selected channels")],
          ]),
          field(
            `${key}.channels`,
            t("Channels"),
            "multi",
            channelOptions(voice).filter(([id]) => id),
          ),
        ],
      ),
    );

  community.append(
    group(
      t("Server settings"),
      t("Choose whether members of this server can track their time with Music."),
      [
        field(
          "statistics",
          t("Allow time tracking"),
          "checkbox",
          [],
          t("Members choose whether to take part. Use Save changes to apply this to the server."),
        ),
      ],
    ),
  );
  const personal = card(
    t("Your activity & privacy"),
    t("Time counts while Music plays in your call and you aren’t deafened. It also contributes to server track totals without your name. Your choices apply immediately."),
    "h3",
  );
  const leaderboard = card(
    t("Server leaderboard"),
    t("Only members who choose to appear are listed."),
    "h3",
  );
  const preferences = el("div");
  const ranking = el("div");
  const loading = el("p", t("Loading your preferences…"), "community-empty");
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
  if (!albumOpen) editor.append(el("h1", pages[page] || pages.overview));
  if (page === "overview") renderOverview(editor);
  if (page === "settings") renderSettings(editor);
  if (page === "stations") {
    stationLibrary(editor);
    if (["night", "rain", "sleep"].some((id) => !catalog.some((s) => s.id === id)))
      editor.append(card(t("More stations"), t("Additional stations are being prepared.")));
  }
  if (page === "profile") {
    renderProfile(editor);
    loadProfile(current, sequence);
  }
  const bar = el("div", undefined, "savebar");
  bar.id = "music-savebar";
  const status = el("p", t("You have unsaved changes"));
  status.id = "music-save-status";
  status.setAttribute("role", "status");
  const discard = button(t("Discard"), () => {
    draft = JSON.parse(baseline);
    persistDraft();
    render();
  });
  discard.id = "music-discard";
  const saveButton = button(t("Save changes"), save, true);
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
    ? t("Choose a different name. Discriminatory slurs and hate slogans are not allowed.") : "";
}
function renderProfile(editor) {
  const c = card(t("Server profile"), t("Edit your profile below. Nothing changes in Discord until you apply."));
  c.classList.add("profile-editor-card");
  if (sharedMusicControlsLocked())
    c.append(premiumCallout(t("A custom avatar and banner need Vozen Premium on this server. The display name is free.")));
  const form = el("form"), surface = el("div", undefined, "discord-profile");
  const view = { form, saved: {}, uploads: {}, pending: 0, loading: true, fields: {}, clear: {} };
  profileView = view;
  form.id = "profile-form";
  form.addEventListener("submit", e => { e.preventDefault(); saveProfile(false).catch(e => notice(e.message, "error")); });
  for (const [key, label, type, max] of [["nick", t("Display name"), "text", 32], ["avatar", t("Avatar"), "file"], ["banner", t("Banner"), "file"]]) {
    const wrap = el("div", undefined, "music-field"), input = el("input"), lab = el("label", label), help = el("small");
    input.name = key; input.id = `profile-${key}`; input.disabled = true;
    input.type = type;
    if (max) input.maxLength = max;
    if (type === "file") { input.accept = "image/png,image/jpeg,image/webp"; input.hidden = true; }
    lab.htmlFor = input.id;
    help.id = `profile-${key}-help`;
    input.setAttribute("aria-describedby", help.id);
    bindMusicText(help, type === "file" ? t("Static PNG, JPEG or WebP · up to 2 MB. Changes are previewed before upload.") : "");
    wrap.append(lab, input, help);
    view.fields[key] = wrap;
    if (type === "file") {
      if (sharedMusicControlsLocked()) wrap.dataset.musicPremiumLocked = "";
      lab.className = "profile-sr-only";
      const clear = button(t(key === "avatar" ? "Undo avatar selection" : "Undo banner selection"), () => { input.value = ""; delete view.uploads[key]; bindMusicText(input, "", "validity"); notice(""); updateProfilePreview(view); });
      clear.className = "profile-image-undo";
      bindMusicText(clear, t(key === "avatar" ? "Undo avatar selection" : "Undo banner selection"), "aria-label"); bindMusicText(clear, t(key === "avatar" ? "Undo avatar selection" : "Undo banner selection"), "title"); clear.textContent = "↶";
      clear.hidden = true; view.clear[key] = clear;
      clear.disabled = true;
      const trigger = button("", () => input.click());
      trigger.disabled = true; trigger.className = "profile-image-trigger";
      bindMusicText(trigger, t(key === "avatar" ? "Edit avatar" : "Edit banner"), "aria-label");
      const pencil = el("span", "✎", "profile-pencil"); pencil.setAttribute("aria-hidden", "true");
      trigger.append(pencil, el("span", t(key === "avatar" ? "Edit avatar" : "Edit banner"), "profile-edit-label"));
      const area = el("div", undefined, `profile-${key}-editor`);
      const image = el("img", undefined, `profile-${key}`);
      bindMusicText(image, t("{label} preview", { label }), "alt");
      if (key === "avatar") image.src = "/music/avatar.png"; else image.hidden = true;
      view[key] = image;
      area.append(image, trigger, clear);
      wrap.prepend(area);
      input.addEventListener("change", async () => {
        notice("");
        delete view.uploads[key]; bindMusicText(input, "", "validity");
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
          if (profileView === view && input.files[0] === file) bindMusicText(input, t(e.message || "Choose a valid image."), "validity");
        } finally { view.pending--; updateProfilePreview(view); }
      });
    } else input.addEventListener("input", () => { notice(""); updateProfilePreview(view); });
  }
  const body = el("div", undefined, "profile-preview-body");
  const name = el("div", undefined, "profile-preview-name");
  name.append(view.fields.nick, el("span", "APP", "profile-app-badge"));
  view.bio = el("p", "", "profile-description");
  view.bio.hidden = true;
  body.append(name, view.bio, el("p", t("Server profile · Preview"), "profile-server-label"));
  surface.append(view.fields.banner, view.fields.avatar, body);
  form.append(surface, el("p", t("Select the banner or avatar to change it. Static PNG, JPEG or WebP, up to 2 MB. This preview approximates Discord."), "help"));
  view.apply = button(t("Apply profile"), () => saveProfile(false), true); view.apply.disabled = true;
  const actions = el("div", undefined, "profile-actions");
  actions.append(view.apply, button(t("Restore Vozen profile"), () => { if (confirm(t("Restore the default bot profile in this server?"))) return saveProfile(true); }));
  form.append(el("p", t("Applies to this server only. Discriminatory names are not allowed."), "help"), actions);
  view.error = el("p", "", "profile-error"); view.error.setAttribute("role", "status"); view.error.hidden = true;
  view.retry = button(t("Retry loading profile"), () => loadProfile(current, sequence)); view.retry.hidden = true;
  form.append(view.error, view.retry);
  c.append(form); editor.append(c);
}
function updateProfilePreview(view) {
  if (profileView !== view || !view.form.isConnected) return;
  const nick = $("profile-nick");
  const message = profileTextError(nick.value, view.saved.policy) || (nick.value && (!nick.value.trim() || /[\u0000-\u001f\u007f]/.test(nick.value)) ? t("Use a visible name without line breaks.") : "");
  bindMusicText(nick, message, "validity");
  nick.setAttribute("aria-invalid", String(!!message));
  bindMusicText($("profile-nick-help"), t("{length}/{limit} characters · Leave empty for the default name.", { length: [...nick.value].length, limit: nick.maxLength }));
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
  bindMusicText(view.error, invalid
    ? musicValidityMessage(invalid)
    : view.pending ? t("Preparing your image preview…") : "");
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
            !confirm(t(payload.erase_statistics ? "Do you want to delete your recorded time in this server?" : "Do you want to delete your likes and dislikes in this server?"))) return;
        pending = true;
        control.disabled = true;
        try {
          await api(`/api/guilds/${id}/personal`, {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(payload),
          });
          if (connected()) {
            notice(t("Your preferences were saved."), "success");
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
      el("h4", t("My time tracking")),
      el("p", t("{status} · {minutes} min recorded", { status: t(enabled ? (serverEnabled ? "On" : "Paused by server") : "Off"), minutes: Math.floor(me.seconds / 60) })),
    );
    tracking.append(trackingCopy, action(
      enabled ? t("Turn off tracking") : t("Turn on tracking"),
      { privacy: { enabled: !enabled, visible: false } },
      "community-tracking",
    ));
    const visibility = el("div", undefined, "community-row");
    const visibilityCopy = el("div");
    visibilityCopy.append(
      el("h4", t("My leaderboard visibility")),
      el("p", visible
        ? (serverEnabled ? t("Visible to this server") : t("Set to visible when the server leaderboard is available"))
        : enabled ? t("Hidden from the leaderboard") : t("Turn on tracking to choose who can see your time.")),
    );
    visibility.append(visibilityCopy);
    if (enabled) visibility.append(action(
      visible ? t("Hide my entry") : t("Show my entry"),
      { privacy: { enabled: true, visible: !visible } },
      "community-visibility",
    ));
    const removal = el("details", undefined, "settings-data-removal");
    removal.append(el("summary", t("Delete your data")));
    const removalActions = el("div", undefined, "settings-actions");
    removalActions.append(
      action(t("Erase my recorded time"), { erase_statistics: true }),
      action(t("Delete my likes and dislikes"), { clear_ratings: true }),
    );
    removal.append(el("p", t("Erasing your time also turns off tracking. Your song ratings can be deleted separately.")), removalActions);
    panel.replaceChildren(tracking, visibility, removal);
    if (focusId) $(focusId)?.focus({ preventScroll: true });

    const list = el("ol", undefined, "community-leaderboard");
    for (const [user, seconds] of ranking.rows) {
      const row = el("li");
      const entry = el("div");
      const member = el("span", user === viewerId ? t("You") : t("Member"));
      if (user !== viewerId) member.append(el("small", t("Discord ID {id}", { id: user })));
      entry.append(
        member,
        el("strong", t("{minutes} min", { minutes: Math.floor(seconds / 60) })),
      );
      row.append(entry);
      list.append(row);
    }
    leaderboard.replaceChildren(ranking.rows.length ? list : el("p",
      serverEnabled ? t("No members have joined the leaderboard yet.") : t("Time tracking is turned off for this server."),
      "community-empty",
    ));
  } catch (e) {
    if (connected()) {
      const error = el("p", t(e.message));
      error.setAttribute("role", "status");
      panel.replaceChildren(error, button(t("Retry"), () => loadPersonal(id, seq, panel, leaderboard)));
      leaderboard.replaceChildren(el("p", t("The leaderboard could not load."), "community-empty"));
    }
  }
}
async function loadProfile(id, seq) {
  const view = profileView;
  let placeholder;
  if (view.loading) {
    placeholder = el("div", undefined, "workspace-skeleton");
    placeholder.setAttribute("role", "status");
    const shape = document.querySelector('#workspace-loading [data-shape="profile"]').cloneNode(true);
    shape.removeAttribute("data-shape");
    shape.setAttribute("aria-hidden", "true");
    placeholder.append(el("span", t("Loading server profile"), "vp-sr"), shape);
    view.form.hidden = true;
    view.form.before(placeholder);
  }
  try {
    const p = await api(`/api/guilds/${id}/profile`);
    if (seq !== sequence || page !== "profile" || view !== profileView) return false;
    view.saved = p;
    $("profile-nick").value = p.nick || "";
    for (const input of view.form.querySelectorAll("input,textarea,button")) {
      input.disabled = !!view.saving || !!input.closest("[data-music-premium-locked]");
    }
    view.loading = false; view.retry.hidden = true;
    updateProfilePreview(view);
    return true;
  } catch (e) {
    if (seq === sequence && page === "profile" && view === profileView) { notice(e.message, "error"); view.retry.hidden = false; }
    return false;
  } finally {
    if (placeholder) {
      placeholder.remove();
      view.form.hidden = false;
    }
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
      notice(t("Discord applied the profile to this server."), "success");
      if (await loadProfile(id, sequence)) {
        view.uploads = {};
        for (const key of ["avatar", "banner"]) $(`profile-${key}`).value = "";
      }
    }
  } finally {
    view.saving = false;
    buttons.forEach((b) => (b.disabled = view.loading || !!b.closest("[data-music-premium-locked]")));
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
  Array.from(select.options).forEach(option => option.remove());
  select.removeAttribute("title");
  for (const s of serverList) {
    const option = new Option("", s.id, false, s.id === current);
    const name = document.createElement("span");
    name.className = "server-option-name";
    name.textContent = s.name;
    option.append(serverAvatar(s, "server-option-icon"), name);
    select.add(option);
    if (s.id === current) {
      select.title = s.name;
      const icon = serverAvatar(s, "server-option-icon");
      icon.id = "selected-server-icon";
      select.before(icon);
    }
  }
  const choose = new Option("", "__choose_server__");
  bindMusicText(choose, t("Choose another server"));
  select.add(choose);
  select.disabled = saving;
}
function showServerPicker() {
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
  acceptLocation(url);
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
  $("server-picker").hidden = false;
  $("main").scrollTop = 0;
  $("main").focus();
}
async function loadServer(id) {
  const seq = ++sequence;
  stopBrowserPreview();
  current = id;
  notice("");
  baseline = "";
  draft = null;
  $("server-picker").hidden = true;
  $("sidebar").hidden = false;
  renderServers();
  for (const control of $("music-pages").querySelectorAll("button"))
    control.disabled = true;
  $("editor").hidden = true;
  $("empty").hidden = true;
  const url = new URL(location.href);
  url.pathname = `/music/${id}`;
  url.searchParams.delete("server");
  acceptLocation(url);
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
        localStorage.getItem(draftKey(id)) || "null",
      );
      if (
        saved &&
        saved.revision === draft.revision &&
        validDraft(saved, data.settings)
      ) {
        draft = saved;
        let dropped = false;
        if (sharedMusicControlsLocked()) {
          // Locked Premium settings cannot be saved here: keep their saved values.
          for (const key of [...premiumFields, "voice_rules", "text_rules"]) {
            if (JSON.stringify(draft[key]) !== JSON.stringify(data.settings[key])) {
              draft[key] = structuredClone(data.settings[key]);
              dropped = true;
            }
          }
        }
        notice(dropped
          ? t("Your unsaved draft was restored without its changes to Premium settings.")
          : t("Your unsaved draft was restored."));
      } else if (saved) {
        notice(
          t("An older browser draft exists. Server settings changed; it was not applied."),
        );
        // Tell the user once; a refused draft would otherwise warn on every visit.
        localStorage.removeItem(draftKey(id));
      }
    } catch {
      notice(t("This browser could not restore your draft."));
    }
    const legacySection = Object.hasOwn(settingsSections, page) ? page : null;
    if (legacySection) page = "settings";
    if (!Object.hasOwn(pages, page)) page = "overview";
    $("editor").hidden = false;
    if (page === "overview") url.searchParams.delete("page");
    else if (url.searchParams.has("page")) url.searchParams.set("page", page);
    if (legacySection) url.searchParams.set("section", legacySection);
    acceptLocation(url);
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
      if (!$("empty").querySelector(".install")) {
        const invite = $("picker-invite").cloneNode(true);
        invite.removeAttribute("id");
        invite.classList.add("install");
        $("empty").append(invite);
      }
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
  const destination = e.target.value;
  e.target.value = current;
  if (!canLeaveServer()) return;
  if (destination === "__choose_server__") showServerPicker();
  else if (destination !== current) loadServer(destination);
});
$("product-switcher").addEventListener("change", (e) => {
  if (
    saving ||
    (changed() && !confirm(t("Your draft is kept in this browser. Leave Music?")))
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
$("langPanel").addEventListener("click", event => {
  const option = event.target.closest("[data-lang]");
  if (!option || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  event.preventDefault();
  const code = option.dataset.lang;
  if (code !== musicLanguage) {
    const url = new URL(location.href);
    url.searchParams.set("lang", code);
    rememberMusicLanguage(code);
    applyMusicLanguage(code);
    acceptLocation(url);
  }
  closeNavigation();
  $("langBtn").focus();
});
window.addEventListener("pagehide", stopBrowserPreview);
window.addEventListener("popstate", () => {
  if (!viewerId) return;
  const wanted = requestedServer();
  if (saving || (wanted !== current && !canLeaveServer())) {
    acceptLocation(acceptedUrl);
    return;
  }
  page = new URLSearchParams(location.search).get("page") || "overview";
  if (!serverList.some((server) => server.id === wanted)) {
    showServerPicker();
    return;
  }
  if (wanted !== current || !draft) {
    loadServer(wanted);
    return;
  }
  if (Object.hasOwn(settingsSections, page)) {
    acceptLocation(pageLocation(page));
    page = "settings";
  } else {
    acceptedUrl = location.href;
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
