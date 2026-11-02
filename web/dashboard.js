"use strict";
const $ = (id) => document.getElementById(id);
let profileView = null;
let csrf = "",
  current = "",
  saving = false,
  sequence = 0,
  serverList = [],
  model = null,
  baseline = "",
  draft = null,
  catalog = [],
  page = new URLSearchParams(location.search).get("page") || "overview",
  step = 0;
const pages = {
  overview: "Overview",
  setup: "Quick Setup",
  playback: "Playback",
  access: "Access & Channels",
  stations: "Stations",
  community: "Community",
  profile: "Bot Profile",
};
const descriptions = {
  overview: "Your Music settings at a glance.",
  setup: "Choose how Music should work, then review and save.",
  playback: "Choose the sound and behaviour of your radio.",
  access: "Choose who can control Music and where it can play.",
  stations: "Instrumental recordings, sources and credits.",
  community: "Optional time-in-call profiles for this server.",
  profile: "Change how Music looks in this server only.",
};
function el(tag, text, cls) {
  const e = document.createElement(tag);
  if (text !== undefined) e.textContent = text;
  if (cls) e.className = cls;
  return e;
}
function notice(message, kind = "info") {
  $("notice").textContent = message;
  $("notice").dataset.kind = kind;
  $("notice").hidden = !message;
}
async function api(path, options = {}) {
  const r = await fetch(`/music${path}`, {
    credentials: "same-origin",
    ...options,
    headers: {
      ...options.headers,
      ...(options.method ? { "X-CSRF-Token": csrf } : {}),
    },
  });
  let data;
  try {
    data = await r.json();
  } catch {
    throw Error("The server could not respond. Try again.");
  }
  if (!r.ok) {
    if (r.status === 401) signedOut();
    throw Object.assign(Error(data.error || "The request failed."), {
      status: r.status,
    });
  }
  return data;
}
function signedOut() {
  csrf = "";
  current = "";
  sequence++;
  for (const id of ["editor", "sidebar", "loading", "empty", "server-picker"])
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
function card(title, description) {
  const c = el("section", undefined, "music-card");
  c.append(el("h2", title));
  if (description) c.append(el("p", description, "help"));
  return c;
}
function field(path, label, type = "text", options = [], help = "") {
  const wrap = el("div", undefined, "music-field"),
    id = `field-${path.replaceAll(".", "-")}`;
  const l = el("label", label);
  l.htmlFor = id;
  wrap.append(l);
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
  wrap.append(input);
  if (help) wrap.append(el("small", help));
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
  const c = card(title, description),
    grid = el("div", undefined, "field-grid");
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
    field("default_station", "Default station", "select", stationOptions()),
    field(
      "volume",
      "Default and current volume",
      "number",
      [],
      "0–100%. Track loudness correction is applied separately.",
    ),
    field("radio_mode", "Radio mode", "select", [
      ["session", "Session — leave when empty"],
      ["always", "24/7 — remain connected"],
      ["auto_join", "Auto-join — start when someone enters"],
    ]),
    field(
      "voice_channel",
      "Fixed / Auto-join voice channel",
      "select",
      channelOptions(true),
    ),
    field("announcements", "Show the player in voice chat automatically", "checkbox"),
    field(
      "announcement_channel",
      "Auto-join notification channel",
      "select",
      channelOptions(false),
      "Required for Auto-join. Player controls appear in the voice channel's chat.",
    ),
    field("tips", "Show command tips on the player", "checkbox"),
  ];
}
function accessFields() {
  return [
    field(
      "djs_only",
      "Restrict playback controls to DJs and managers",
      "checkbox",
    ),
    field("player_locked", "Lock the player to DJs and managers", "checkbox"),
    field("dj_roles", "DJ roles", "multi", roles()),
    field("skip.access", "Who can skip?", "select", [
      ["everyone", "Everyone in the call"],
      ["moderators", "Moderators only"],
    ]),
    field("skip.mode", "How does a skip happen?", "select", [
      ["instant", "One click"],
      ["majority", "Majority vote"],
    ]),
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
  if (page === "setup" && step < 3) return;
  const invalid = $("editor").querySelector(":invalid");
  if (invalid) {
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
function navigate(next) {
  if (!pages[next]) next = "overview";
  page = next;
  step = 0;
  const url = new URL(location.href);
  url.searchParams.set("page", page);
  history.replaceState(null, "", url);
  render();
  closeSections(true);
  $("main").focus();
}
function render() {
  const editor = $("editor");
  editor.replaceChildren();
  $("music-pages").replaceChildren();
  for (const [key, label] of Object.entries(pages)) {
    const b = button(label, () => navigate(key));
    b.setAttribute("aria-current", key === page ? "page" : "false");
    $("music-pages").append(b);
  }
  editor.append(
    button("← Choose another server", showServerPicker),
    el("h1", pages[page] || pages.overview),
    el("p", descriptions[page] || descriptions.overview, "intro"),
  );
  if (page === "overview") {
    const setup = card(
      "Make Music yours",
      "Review channels, your station and playback rules together.",
    );
    setup.append(button("Open Quick Setup", () => navigate("setup"), true));
    editor.append(setup);
    const c = card("Your choices");
    const list = el("dl", undefined, "overview-values");
    for (const [k, v] of [
      [
        "Station",
        catalog.find((s) => s.id === draft.default_station)?.name ||
          draft.default_station,
      ],
      ["Volume", `${draft.volume}%`],
      ["Mode", draft.radio_mode],
      ["Skip", `${draft.skip.access} · ${draft.skip.mode}`],
      [
        "Current playback",
        model.session
          ? `${model.session.active === false ? "Stopped" : model.session.paused ? "Paused" : "Playing"} · ${model.session.station} · ${model.session.track}`
          : "No active session",
      ],
    ])
      list.append(el("dt", k), el("dd", v));
    c.append(list);
    editor.append(c);
  }
  if (page === "setup") {
    const titles = [
      "Channel & mode",
      "Station & volume",
      "Permissions",
      "Review & save",
    ];
    editor.append(
      el("p", `Step ${step + 1} of 4 · ${titles[step]}`, "setup-progress"),
    );
    const fields =
      step === 0
        ? [
            field(
              "voice_channel",
              "Voice channel",
              "select",
              channelOptions(true),
            ),
            field(
              "announcement_channel",
              "Auto-join notification channel",
              "select",
              channelOptions(false),
              "Player controls appear in voice chat. This text channel is required for Auto-join.",
            ),
            field("radio_mode", "Radio mode", "select", [
              ["session", "Session"],
              ["always", "24/7"],
              ["auto_join", "Auto-join"],
            ]),
          ]
        : step === 1
          ? [
              field(
                "default_station",
                "Default station",
                "select",
                stationOptions(),
              ),
              field("volume", "Volume", "number"),
            ]
          : step === 2
            ? accessFields()
            : [];
    const c = group(
      titles[step],
      step === 3
        ? "Nothing is saved until you choose Save changes."
        : "Changes remain a draft until the final review.",
      fields,
    );
    if (step === 3)
      c.append(
        el(
          "p",
          `${catalog.find((s) => s.id === draft.default_station)?.name || draft.default_station} · ${draft.volume}% · ${draft.radio_mode} · ${draft.djs_only ? "DJs" : "Everyone"} controls · ${draft.skip.mode} skip`,
        ),
      );
    const actions = el("div", undefined, "setup-actions");
    if (step > 0)
      actions.append(
        button("Back", () => {
          step--;
          render();
        }),
      );
    if (step < 3)
      actions.append(
        button(
          "Continue",
          () => {
            const invalid = editor.querySelector(":invalid");
            if (invalid) {
              invalid.reportValidity();
              return;
            }
            if (
              step === 0 &&
              draft.radio_mode === "auto_join" &&
              (!draft.voice_channel || !draft.announcement_channel)
            )
              throw Error(
                "Choose both a voice channel and text channel for Auto-join.",
              );
            step++;
            render();
          },
          true,
        ),
      );
    else actions.append(button("Save changes", save, true));
    c.append(actions);
    editor.append(c);
  }
  if (page === "playback") {
    editor.append(
      group(
        "Radio settings",
        "Session leaves immediately when the last person leaves. Bots do not count.",
        playbackFields(),
      ),
    );
    const timer = card(
      "Sleep timer",
      "Use /sleep minutes:30 in your call. Use /sleep cancel:true to cancel.",
    );
    timer.append(
      el(
        "p",
        model.session?.sleep_until
          ? `Current deadline: ${new Date(model.session.sleep_until * 1000).toLocaleString()}`
          : "No active sleep timer in this snapshot.",
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
    editor.append(timer);
  }
  if (page === "access") {
    editor.append(
      group(
        "Playback & skip",
        "DJ restrictions do not override skip voting thresholds.",
        accessFields(),
      ),
    );
    for (const [key, label, voice] of [
      ["voice_rules", "Voice channels", true],
      ["text_rules", "Text channels", false],
    ])
      editor.append(
        group(
          label,
          "An empty blocklist allows all channels; an empty allowlist allows none.",
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
  }
  if (page === "stations") {
    editor.append(
      card(
        "My Station",
        "Four favorite selections for every discovery selection, using the people currently in the call. Each person has equal weight; dislikes exclude recordings. Use /like, /dislike and /library.",
      ),
    );
    for (const s of catalog.filter((s) => s.id !== "my")) {
      const c = card(s.name, `${s.tracks.length} recordings`);
      const details = el("details"),
        summary = el("summary", "Track credits");
      details.append(summary);
      const list = el("ul");
      for (const t of s.tracks) {
        const li = el("li");
        const link = el("a", `${t.title} — ${t.artist}`);
        if (t.source.startsWith("https://")) {
          link.href = t.source;
          link.target = "_blank";
          link.rel = "noopener noreferrer";
        }
        li.append(link, el("small", ` · ${t.license}`));
        list.append(li);
      }
      details.append(list);
      c.append(details);
      editor.append(c);
    }
    if (["night", "rain", "sleep"].some((id) => !catalog.some((s) => s.id === id)))
      editor.append(card("More stations", "Additional stations are being prepared."));
  }
  if (page === "community") {
    editor.append(
      group(
        "Server statistics",
        "Individual collection requires each person to opt in with /privacy. No global leaderboard.",
        [
          field(
            "statistics",
            "Allow optional time-in-call statistics",
            "checkbox",
          ),
        ],
      ),
    );
    const personal = card(
      "Your privacy & this server leaderboard",
      "Time in call while Music is playing, excluding deafened time. This is not proof of listening.",
    );
    personal.id = "personal-panel";
    editor.append(personal);
    loadPersonal(current, sequence);
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
  if (page !== "setup" || step === 3) editor.append(bar);
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
    ? "Choose a different name or description. Discriminatory slurs and hate slogans are not allowed." : "";
}
function renderProfile(editor) {
  const c = card("Server profile", "Personalise Music for this server. Review the preview before applying changes.");
  const layout = el("div", undefined, "profile-layout"), form = el("form"), preview = el("aside", undefined, "profile-preview");
  const view = { form, saved: {}, uploads: {}, pending: 0, loading: true };
  profileView = view;
  form.id = "profile-form";
  form.addEventListener("submit", e => { e.preventDefault(); saveProfile(false).catch(e => notice(e.message, "error")); });
  for (const [key, label, type, max] of [["nick", "Display name", "text", 32], ["bio", "Description", "textarea", 190], ["avatar", "Avatar", "file"], ["banner", "Banner", "file"]]) {
    const wrap = el("div", undefined, "music-field"), input = el(type === "textarea" ? "textarea" : "input"), lab = el("label", label), help = el("small");
    input.name = key; input.id = `profile-${key}`; input.disabled = true;
    if (type !== "textarea") input.type = type;
    if (max) input.maxLength = max;
    if (type === "file") input.accept = "image/png,image/jpeg,image/webp";
    lab.htmlFor = input.id;
    help.id = `profile-${key}-help`;
    input.setAttribute("aria-describedby", help.id);
    help.textContent = type === "file" ? "Static PNG, JPEG or WebP · up to 2 MB. Changes are previewed before upload." : "";
    wrap.append(lab, input, help);
    if (type === "file") {
      const clear = button("Clear selection", () => { input.value = ""; delete view.uploads[key]; input.setCustomValidity(""); notice(""); updateProfilePreview(view); });
      clear.disabled = true;
      wrap.append(clear);
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
    form.append(wrap);
  }
  view.apply = button("Apply profile", () => saveProfile(false), true); view.apply.disabled = true;
  const actions = el("div", undefined, "profile-actions");
  actions.append(view.apply, button("Restore Vozen profile", () => { if (confirm("Restore the default bot profile in this server?")) return saveProfile(true); }));
  form.append(el("p", "Applies to this server only. Discriminatory names and descriptions are not allowed.", "help"), actions);
  preview.append(el("h3", "Profile preview"), el("p", "Approximate Discord appearance. Nothing changes until you apply.", "help"));
  const surface = el("div", undefined, "discord-profile"), banner = el("div", undefined, "profile-banner");
  view.banner = el("img"); view.banner.alt = "Banner preview"; view.banner.hidden = true; banner.append(view.banner);
  view.avatar = el("img", undefined, "profile-avatar"); view.avatar.alt = "Avatar preview"; view.avatar.src = "/music/avatar.png";
  const body = el("div", undefined, "profile-preview-body"), name = el("div", undefined, "profile-preview-name");
  view.name = el("strong", "Vozen Music"); name.append(view.name, el("span", "APP", "profile-app-badge"));
  view.bio = el("p", "No description", "profile-preview-bio");
  body.append(name, view.bio, el("p", "Server profile", "profile-server-label"));
  surface.append(banner, view.avatar, body); preview.append(surface);
  view.error = el("p", "", "profile-error"); view.error.setAttribute("role", "status"); view.error.hidden = true;
  view.retry = button("Retry loading profile", () => loadProfile(current, sequence)); view.retry.hidden = true;
  preview.append(view.error, view.retry);
  layout.append(form, preview); c.append(layout); editor.append(c);
}
function updateProfilePreview(view) {
  if (profileView !== view || !view.form.isConnected) return;
  const nick = $("profile-nick"), bio = $("profile-bio");
  for (const input of [nick, bio]) {
    const message = profileTextError(input.value, view.saved.policy) || (input === nick && input.value && (!input.value.trim() || /[\u0000-\u001f\u007f]/.test(input.value)) ? "Use a visible name without line breaks." : "");
    input.setCustomValidity(message);
    input.setAttribute("aria-invalid", String(!!message));
    $(`${input.id}-help`).textContent = `${[...input.value].length}/${input.maxLength} characters${input === nick ? " · Leave empty for the default name." : ""}`;
  }
  view.name.textContent = nick.value || view.saved.default_name || "Vozen Music";
  view.bio.textContent = bio.value || "No description";
  const image = key => {
    if (view.uploads[key]) return view.uploads[key];
    const hash = view.saved[key], bot = view.saved.bot_id;
    return /^(a_)?[a-f0-9]{32}$/.test(hash || "") && /^\d+$/.test(bot || "") && /^\d+$/.test(current)
      ? `https://cdn.discordapp.com/guilds/${current}/users/${bot}/${key}s/${hash}.png?size=512` : "";
  };
  view.avatar.src = image("avatar") || view.saved.default_avatar_url || "/music/avatar.png";
  const banner = image("banner");
  view.banner.hidden = !banner;
  if (banner) view.banner.src = banner; else view.banner.removeAttribute("src");
  const invalid = [...view.form.querySelectorAll("input,textarea")].find(i => !i.validity.valid);
  view.error.textContent = invalid?.validationMessage || (view.pending ? "Preparing your image preview…" : "");
  view.error.hidden = !view.error.textContent;
  view.apply.disabled = view.loading || !!view.pending || !!view.saving || !!invalid;
}
async function readProfileFile(file) {
  if (file.size > 2 * 1024 * 1024) throw Error("Images must be no larger than 2 MB.");
  if (!["image/png", "image/jpeg", "image/webp"].includes(file.type)) throw Error("Choose a PNG, JPEG or WebP image.");
  return new Promise((resolve, reject) => {
    const reader = new FileReader(); reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(Error("The image could not be read.")); reader.readAsDataURL(file);
  });
}
async function loadPersonal(id, seq) {
  try {
    const [me, ranking] = await Promise.all([
      api(`/api/guilds/${id}/personal`),
      api(`/api/guilds/${id}/community`),
    ]);
    if (seq !== sequence || current !== id || page !== "community") return;
    const c = $("personal-panel");
    c.append(
      el(
        "p",
        `${Math.floor(me.seconds / 60)} minutes collected. Collection ${me.privacy.enabled ? "on" : "off"}; leaderboard ${me.privacy.visible ? "visible" : "hidden"}.`,
      ),
    );
    for (const [label, payload] of [
      ["Opt in", { privacy: { enabled: true, visible: false } }],
      ["Show on leaderboard", { privacy: { enabled: true, visible: true } }],
      ["Hide from leaderboard", { privacy: { ...me.privacy, visible: false } }],
      ["Disable collection", { privacy: { enabled: false, visible: false } }],
      ["Erase statistics", { erase_statistics: true }],
      ["Delete my favorites and dislikes", { clear_ratings: true }],
    ])
      c.append(
        button(label, async () => {
          if (
            (payload.erase_statistics || payload.clear_ratings) &&
            !confirm(`${label} in this server?`)
          )
            return;
          await api(`/api/guilds/${id}/personal`, {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(payload),
          });
          if (id === current) {
            notice("Your preferences were saved.", "success");
            render();
          }
        }),
      );
    const list = el("ol");
    for (const [user, seconds] of ranking.rows)
      list.append(el("li", `Member ${user} · ${Math.floor(seconds / 60)} min`));
    c.append(list);
  } catch (e) {
    if (id === current) notice(e.message, "error");
  }
}
async function loadProfile(id, seq) {
  const view = profileView;
  try {
    const p = await api(`/api/guilds/${id}/profile`);
    if (seq !== sequence || page !== "profile" || view !== profileView) return false;
    view.saved = p;
    $("profile-nick").value = p.nick || "";
    $("profile-bio").value = p.bio || "";
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
    body.bio = $("profile-bio").value;
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
function renderServers() {
  const select = $("server-select");
  select.replaceChildren();
  for (const s of serverList)
    select.add(new Option(s.name, s.id, false, s.id === current));
  select.disabled = saving;
}
function showServerPicker() {
  if (saving) return;
  if (current && !persistDraft() && changed()) return;
  sequence++;
  current = "";
  draft = null;
  baseline = "";
  closeSections(false);
  for (const id of ["editor", "sidebar", "loading", "empty"])
    $(id).hidden = true;
  notice("");
  const url = new URL(location.href);
  url.searchParams.delete("server");
  url.searchParams.delete("page");
  history.replaceState(null, "", url);
  const choices = $("server-choices");
  choices.replaceChildren();
  for (const server of serverList) {
    const row = button("", () => {
      page = "overview";
      return loadServer(server.id);
    });
    row.className = "server-choice vp-server";
    const avatar = el("span", server.name.slice(0, 1).toUpperCase(), "server-avatar vp-avatar");
    if (/^\d+$/.test(server.id) && /^(a_)?[a-f0-9]{32}$/.test(server.icon || "")) {
      const image = el("img");
      image.alt = "";
      image.src = `https://cdn.discordapp.com/icons/${server.id}/${server.icon}.png?size=64`;
      image.addEventListener("error", () => image.remove());
      avatar.append(image);
    }
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
  baseline = "";
  draft = null;
  $("server-picker").hidden = true;
  $("sidebar").hidden = false;
  renderServers();
  $("editor").hidden = true;
  $("empty").hidden = true;
  $("loading").hidden = false;
  try {
    const [data, stations] = await Promise.all([
      api(`/api/guilds/${id}/server-settings`),
      api("/api/stations"),
    ]);
    if (seq !== sequence) return;
    model = data;
    catalog = stations.stations;
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
    if (!pages[page]) page = "overview";
    $("editor").hidden = false;
    const url = new URL(location.href);
    url.searchParams.set("server", id);
    history.replaceState(null, "", url);
    render();
  } catch (e) {
    if (seq === sequence) notice(e.message, "error");
  } finally {
    if (seq === sequence) $("loading").hidden = true;
  }
}
async function refreshServers() {
  try {
    const data = await api("/api/guilds");
    serverList = data.guilds;
    renderServers();
    if (!serverList.length) {
      if (!$("empty").querySelector(".install"))
        $("empty").append($("sidebar").querySelector('a[href*="discord.com/oauth2"]').cloneNode(true));
      $("empty").hidden = false;
      return;
    }
    const wanted = new URLSearchParams(location.search).get("server");
    if (serverList.some((s) => s.id === wanted)) await loadServer(wanted);
    else showServerPicker();
  } catch (e) {
    notice(e.message, "error");
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
(async () => {
  try {
    const me = await api("/api/me");
    csrf = me.csrf;
    renderAccount(me.user);
    await refreshServers();
  } catch (e) {
    signedOut();
    if (e.status === 401) {
      location.replace("https://vozen.org/account/?music=1");
      return;
    }
    notice(e.message, "error");
  }
})();
