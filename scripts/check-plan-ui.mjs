// Checks how the dashboard marks Vozen plans against local preview servers.
// The locked list must match ServerSettings::changes_shared_music_controls.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import path from "node:path";

const { chromium } = await import(process.env.VOZEN_PLAYWRIGHT_MODULE || "playwright");
const script = path.join(import.meta.dirname, "preview.mjs");
const base = Number(process.env.VOZEN_PLAN_CHECK_PORT || 4821);
const plans = ["free", "plus", "premium", "unrestricted"];
const servers = plans.map((plan, i) =>
  spawn(process.execPath, [script, `--plan=${plan}`, `--port=${base + i}`, ...(plan === "free" ? ["--vote=1"] : [])], { stdio: "ignore" }));
const premiumIds = ["field-default_station", "field-volume", "field-voice_channel", "field-announcements",
  "field-announcement_channel", "field-djs_only", "field-player_locked", "field-dj_roles",
  "field-voice_rules-allow", "field-voice_rules-channels", "field-text_rules-allow", "field-text_rules-channels"];

async function open(page, plan, query) {
  await page.goto(`http://localhost:${base + plans.indexOf(plan)}/music/1${query}`);
  await page.locator("#editor").waitFor({ state: "visible" });
}
async function fieldState(page) {
  return page.evaluate((ids) => Object.fromEntries(ids.map((id) => {
    const node = document.getElementById(id);
    const wrap = node?.closest(".music-field");
    const controls = node?.matches("input,select") ? [node] : [...(node?.querySelectorAll("input") || [])];
    return [id, {
      present: !!node,
      locked: wrap?.hasAttribute("data-music-premium-locked") ?? false,
      disabled: controls.length > 0 && controls.every((c) => c.disabled),
      pill: wrap?.querySelector(".music-plan-pill")?.textContent ?? null,
    }];
  })), premiumIds);
}
async function settingsState(page, plan) {
  const state = {};
  for (const section of ["playback", "access"]) {
    await open(page, plan, `?page=settings&section=${section}`);
    if (section === "access") await page.locator(".settings-channel-rules summary").click();
    for (const [id, value] of Object.entries(await fieldState(page))) if (value.present) state[id] = value;
  }
  return state;
}

await new Promise((r) => setTimeout(r, 1200));
const browser = await chromium.launch();
const errors = [];
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  page.on("pageerror", (e) => errors.push(e.message));

  const free = await settingsState(page, "free");
  assert.deepEqual(Object.keys(free).sort(), [...premiumIds].sort(), "every Premium field renders");
  for (const [id, value] of Object.entries(free)) {
    assert.ok(value.locked && value.disabled, `${id} is locked on Free`);
    assert.equal(value.pill, "Premium required", `${id} names the requirement`);
  }
  await open(page, "free", "?page=settings");
  assert.equal(await page.locator(".music-plan-callout a").first().getAttribute("href"), "https://vozen.org/premium#plans");
  assert.match(await page.locator("#field-radio_mode-help").textContent(), /Top\.gg vote/);
  assert.equal(await page.locator("#field-tips").isDisabled(), false, "free settings stay editable");
  await page.locator("#field-tips").click();
  await page.locator("#music-save").click();
  await page.locator("#music-savebar").waitFor({ state: "hidden" });

  const premium = await settingsState(page, "premium");
  for (const [id, value] of Object.entries(premium)) {
    assert.ok(!value.locked && !value.disabled, `${id} is editable on Premium`);
    assert.equal(value.pill, "Premium", `${id} is marked as Premium`);
  }
  assert.equal(await page.locator(".music-plan-callout").count(), 0);

  const plus = await settingsState(page, "plus");
  assert.ok(Object.values(plus).every((v) => v.locked), "Plus does not unlock server settings");
  await open(page, "plus", "?page=settings");
  assert.match(await page.locator(".music-plan-callout p").first().textContent(), /Vozen Plus perks are personal/);
  assert.equal(await page.locator("#field-radio_mode-help").count(), 0, "no vote note without a vote requirement");

  for (const query of ["", "?page=settings", "?page=settings&section=access", "?page=stations", "?page=profile"]) {
    await open(page, "unrestricted", query);
    assert.equal(await page.locator(".music-plan-pill,.music-plan-callout,.overview-plan,[data-music-premium-locked]").count(), 0,
      `no plan markup without plan checks (${query || "overview"})`);
  }
  for (const [plan, title] of [["free", "Vozen Premium isn’t active here"], ["plus", "You have Vozen Plus"], ["premium", "Vozen Premium is active here"]]) {
    await open(page, plan, "");
    assert.equal(await page.locator(".overview-plan h2").textContent(), title);
  }
  await open(page, "free", "?page=profile");
  assert.match(await page.locator(".profile-editor-card .music-plan-callout").textContent(), /avatar and banner/);
  assert.deepEqual(errors, []);
  console.log("Plan UI: Free/Plus locks match the server list, free fields still save, Premium editable, no plan markup when plans are off, overview and profile copy passed.");
} finally {
  await browser.close();
  servers.forEach((s) => s.kill());
}
