// Checks dashboard client state against a local preview server: drafts that
// clear a channel, saved settings kept apart from the draft, and sign-out.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import path from "node:path";

const { chromium } = await import(process.env.VOZEN_PLAYWRIGHT_MODULE || "playwright");
const port = Number(process.env.VOZEN_CLIENT_STATE_PORT || 4851);
const base = `http://localhost:${port}`;
const server = spawn(process.execPath, [path.join(import.meta.dirname, "preview.mjs"), `--port=${port}`], { stdio: "ignore" });

// Wait until the preview server answers instead of guessing a start-up delay.
for (let attempt = 0; ; attempt++) {
  try {
    if ((await fetch(`${base}/music/api/me`)).ok) break;
  } catch {}
  if (attempt > 100) throw new Error("preview server did not start");
  await new Promise((r) => setTimeout(r, 100));
}

async function open(page, query) {
  await page.goto(`${base}/music/1${query}`);
  await page.locator("#editor").waitFor({ state: "visible" });
}
async function saveDraft(page) {
  await page.locator("#music-save").click();
  await page.locator("#music-savebar").waitFor({ state: "hidden" });
}

const browser = await chromium.launch();
const errors = [];
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  page.on("pageerror", (e) => errors.push(e.message));

  // A draft that clears a saved channel survives a reload.
  await open(page, "?page=settings");
  await page.locator("#field-voice_channel").selectOption("4");
  await saveDraft(page);
  await page.locator("#field-voice_channel").selectOption("");
  await page.reload();
  await page.locator("#editor").waitFor({ state: "visible" });
  assert.match(await page.locator("#notice").textContent(), /Your unsaved draft was restored\./);
  assert.equal(await page.locator("#field-voice_channel").inputValue(), "");
  await page.locator("#music-discard").click();

  // After a save, unsaved edits must not leak into the saved settings.
  await open(page, "?page=settings");
  await page.locator("#field-tips").click();
  await saveDraft(page);
  await page.locator('a[data-settings-section="community"]').click();
  await page.locator("#field-statistics").click();
  await page.locator('#music-pages button[data-page="overview"]').click();
  await page.locator(".overview-stations").waitFor({ state: "visible" });
  assert.match(await page.locator(".overview-stations").textContent(), /Enable time tracking in Community/);
  await page.locator("#music-discard").click();

  // Sign-out leaves the editor even though the server answers 204 without a body.
  await open(page, "");
  await page.locator("#logout").click();
  await page.locator("#welcome").waitFor({ state: "visible" });
  assert.equal(
    await page.locator("#notice").evaluate((n) => n.hidden || n.dataset.kind !== "error"),
    true,
    "sign-out must not show an error",
  );

  assert.deepEqual(errors, []);
  console.log("Client state: sign-out, cleared-channel drafts and saved-settings separation passed.");
} finally {
  await browser.close();
  server.kill();
}
