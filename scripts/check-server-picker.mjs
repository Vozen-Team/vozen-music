import assert from "node:assert/strict";
const { chromium } = await import(process.env.VOZEN_PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch();
const dashboard = (process.env.VOZEN_PREVIEW_URL || "http://localhost:4783") + "/music/dashboard";
try {
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  const errors = [];
  page.on("pageerror", e => errors.push(e.message));
  page.on("dialog", dialog => dialog.accept());
  const iconHash = "a".repeat(32);
  const iconUrl = `https://cdn.discordapp.com/icons/2/${iconHash}.png?size=64`;
  let guilds = [{ id: "1", name: "Example server" }, { id: "2", name: "Another server with a long name to check wrapping", icon: iconHash }];
  await page.route(iconUrl, route => route.fulfill({ path: "assets/brand/v4/vozen-music-avatar.png" }));
  let settingsRequests = 0;
  let viewerId = "1";
  let releaseServerSettings;
  let settingsGate;
  let releaseSave;
  let saveGate;
  await page.route("**/music/api/me", route =>
    route.fulfill({ json: { csrf: "test", user: { id: viewerId, username: "Test manager" } } }));
  await page.route("**/music/api/guilds", route => route.fulfill({ json: { guilds } }));
  await page.route("**/music/api/guilds/*/server-settings", async route => {
    settingsRequests++;
    if (route.request().url().includes("/guilds/2/")) {
      if (route.request().method() === "PUT") {
        await saveGate;
        const settings = route.request().postDataJSON();
        return route.fulfill({ json: { settings: { ...settings, revision: settings.revision + 1 } } });
      }
      await settingsGate;
      return route.fulfill({ response: await route.fetch({ url: route.request().url().replace("/guilds/2/", "/guilds/1/") }) });
    }
    return route.continue();
  });
  await page.goto(dashboard + "?server=unknown");
  await page.locator("#server-picker").waitFor({ state: "visible" });
  assert.equal(await page.locator(".server-choice").count(), 2);
  assert.equal(settingsRequests, 0, "No server is silently selected");
  await page.locator(".server-choice").first().click();
  await page.getByRole("heading", { name: "Overview", exact: true }).waitFor();
  assert.equal(page.url(), new URL("/music/1", dashboard).href);
  const select = page.locator("#server-select");
  const albumHeader = page.locator("#editor .station-album-header");
  const icon = page.locator("#selected-server-icon");
  assert.equal(await icon.getAttribute("data-initials"), "E");
  await select.selectOption("2");
  await icon.locator("img").evaluate(img => img.decode());
  assert.equal(await icon.locator("img").getAttribute("src"), iconUrl);
  await select.selectOption("1");
  assert.equal(await icon.getAttribute("data-initials"), "E");
  assert.equal(await icon.locator("img").count(), 0);
  await page.getByRole("heading", { name: "Overview", exact: true }).waitFor();
  await page.locator("#sections-trigger").click();
  await page.locator("#music-pages").getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByLabel("Default and current volume", { exact: true }).fill("42");
  assert.equal(await select.locator("option").last().textContent(), "Choose another server");
  assert.equal(await page.getByRole("button", { name: "Choose another server" }).count(), 0);
  await select.selectOption("__choose_server__");
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem("music-draft:1:1")).volume), 42);
  await page.reload();
  await page.locator("#server-picker").waitFor({ state: "visible" });
  await page.locator(".server-choice").first().click();
  await page.getByRole("heading", { name: "Overview", exact: true }).waitFor();
  assert.match(await page.locator("#notice").innerText(), /draft was restored/);
  await page.goto(dashboard + "?server=1&page=playback");
  await page.getByLabel("Default and current volume", { exact: true }).waitFor();
  assert.equal(new URL(page.url()).pathname, "/music/1");
  assert.equal(new URL(page.url()).searchParams.has("server"), false);
  assert.equal(new URL(page.url()).searchParams.get("page"), "settings");
  assert.equal(new URL(page.url()).searchParams.get("section"), "playback");
  assert.equal(await page.locator("#server-picker").isVisible(), false);
  assert.equal(await page.getByLabel("Default and current volume", { exact: true }).inputValue(), "42");
  await page.evaluate(() => { Storage.prototype.setItem = () => { throw Error("unavailable"); }; });
  await page.getByLabel("Default and current volume", { exact: true }).fill("43");
  await select.selectOption("2");
  assert.equal(await select.inputValue(), "1", "Direct switch must also protect the unsaved draft");
  assert.equal(await page.getByLabel("Default and current volume", { exact: true }).inputValue(), "43");
  await select.selectOption("__choose_server__");
  assert.equal(await select.inputValue(), "1");
  assert.equal(await page.locator("#editor").isVisible(), true, "Keep draft open if storage fails");
  await page.goto(dashboard + "?server=1");
  await page.getByRole("heading", { name: "Overview", exact: true }).waitFor();
  await page.locator("#music-discard").click();
  await page.goto(dashboard + "?server=1&page=stations&album=drum-and-bass");
  await albumHeader.waitFor();
  await select.selectOption("__choose_server__");
  await page.locator("#server-picker").waitFor({ state: "visible" });
  assert.equal(new URL(page.url()).pathname, "/music");
  assert.equal(new URL(page.url()).search, "");

  const volume = page.getByLabel("Default and current volume", { exact: true });
  await page.goto(dashboard + "?server=1&page=settings");
  await volume.waitFor();
  const cleanVolume = await volume.inputValue();
  const ownVolume = cleanVolume === "57" ? "58" : "57";
  await volume.fill(ownVolume);
  await page.evaluate(() => localStorage.setItem("music-draft:1", localStorage.getItem("music-draft:1:1")));
  viewerId = "8";
  await page.reload();
  await volume.waitFor();
  assert.equal(await volume.inputValue(), cleanVolume);
  assert.equal(await page.evaluate(() => localStorage.getItem("music-draft:1") !== null), true);
  viewerId = "1";
  await page.reload();
  await volume.waitFor();
  assert.equal(await volume.inputValue(), ownVolume);
  await page.locator("#music-discard").click();

  await page.goto(dashboard + "?server=1&page=stations");
  await page.locator(".station-albums").waitFor();
  await page.locator(".station-album").filter({ hasText: "Vozen Rain" }).click();
  await albumHeader.waitFor();
  settingsGate = new Promise(resolve => { releaseServerSettings = resolve; });
  await select.selectOption("2");
  await page.locator("#workspace-loading").waitFor();
  const settingsButton = page.locator("#music-pages").getByRole("button", { name: "Settings", exact: true });
  assert.equal(await settingsButton.isDisabled(), true);
  await settingsButton.evaluate(button => button.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  assert.equal(await page.locator("#editor").isHidden(), true);
  releaseServerSettings();
  await albumHeader.waitFor();
  assert.equal(await settingsButton.isEnabled(), true);
  await page.goBack();
  await page.locator(".station-albums").waitFor();
  assert.equal(await select.inputValue(), "1");
  assert.equal(new URL(page.url()).pathname, "/music/1");
  await page.goForward();
  await albumHeader.waitFor();
  assert.equal(await select.inputValue(), "2");
  assert.equal(new URL(page.url()).pathname, "/music/2");

  await page.locator("#sections-trigger").click();
  await settingsButton.click();
  const originalStorage = await page.evaluateHandle(() => Storage.prototype.setItem);
  await page.evaluate(() => { Storage.prototype.setItem = () => { throw Error("unavailable"); }; });
  await volume.fill(ownVolume);
  const accepted = page.url();
  await page.goBack();
  await page.waitForURL(accepted);
  assert.equal(page.url(), accepted, "Failed draft preservation restores the accepted URL");
  assert.equal(await select.inputValue(), "2");
  assert.equal(await volume.inputValue(), ownVolume);
  await page.evaluate(original => { Storage.prototype.setItem = original; }, originalStorage);
  await originalStorage.dispose();

  // A fresh same-document history entry gives Save a separate destination to reject.
  await page.evaluate(() => {
    const current = location.href;
    history.replaceState(null, "", "/music/1?page=stations");
    history.pushState(null, "", current);
  });
  saveGate = new Promise(resolve => { releaseSave = resolve; });
  const saveStarted = page.waitForRequest(request =>
    request.method() === "PUT" && request.url().endsWith("/guilds/2/server-settings"));
  await page.locator("#music-save").click();
  await saveStarted;
  await page.goBack();
  await page.waitForURL(accepted);
  assert.equal(page.url(), accepted);
  assert.equal(await select.inputValue(), "2");
  releaseSave();
  await page.getByText("Settings saved for this server.", { exact: true }).waitFor();
  await select.selectOption("1");
  await volume.waitFor();
  assert.equal(new URL(page.url()).pathname, "/music/1");

  guilds = [];
  await page.goto(dashboard);
  await page.getByRole("heading", { name: "No servers to manage yet" }).waitFor();
  assert.match(await page.locator("#empty .install").getAttribute("href"), /discord.com\/oauth2\/authorize/);
  assert.equal(await page.locator("#sidebar").isVisible(), false);
  assert.equal(await page.locator("#server-picker").isVisible(), false);
  assert.deepEqual(errors, []);
  console.log("Server picker: multiple/empty guilds, invalid/direct links, keyboard entry, drafts and storage failure passed.");
} finally {
  await browser.close();
}
