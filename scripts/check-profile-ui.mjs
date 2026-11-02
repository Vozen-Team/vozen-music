import assert from "node:assert/strict";
import fs from "node:fs";
const { chromium } = await import(process.env.VOZEN_PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch();
const policy = JSON.parse(fs.readFileSync("web/profile-policy.json", "utf8"));
const picture = fs.readFileSync("assets/brand/v4/vozen-music-avatar.png");
try {
  for (const width of [320, 360, 390, 430, 768, 900, 1440]) {
    const page = await browser.newPage({ viewport: { width, height: 1000 } });
    const errors = []; page.on("pageerror", e => errors.push(e.message));
    page.on("dialog", dialog => dialog.accept());
    let puts = 0, fail = true;
    let current = { nick: "Vozen Music", bio: "Instrumental radio for your server.", bot_id: "1", default_name: "Vozen Music", default_avatar_url: "/music/avatar.png", policy };
    await page.route("**/music/api/guilds/1/profile", async route => {
      if (route.request().method() === "GET") return route.fulfill({ json: current });
      puts++;
      assert.equal(route.request().headers()["x-csrf-token"], "local-fixture");
      if (fail) return route.fulfill({ status: 502, json: { error: "Discord did not apply the profile. Try again." } });
      const body = route.request().postDataJSON();
      current = body.reset ? { ...current, nick: null, bio: "" } : { ...current, nick: body.nick, bio: body.bio };
      return route.fulfill({ json: { saved: true } });
    });
    await page.goto(`${process.env.VOZEN_PREVIEW_URL || "http://localhost:4783"}/music/dashboard?server=1&page=profile`);
    const name = page.getByLabel("Display name", { exact: true });
    await name.waitFor(); await page.waitForFunction(() => document.getElementById("profile-nick")?.disabled === false);
    for (const text of ["nigger", "N1GG3R", "n.i.g.g.e.r", "ｎｉｇｇｅｒ", "n\u200bigger"]) {
      await name.fill(text);
      assert.equal(await page.getByRole("button", { name: "Apply profile", exact: true }).isDisabled(), true);
      assert.match(await page.locator(".profile-error").innerText(), /not allowed/);
    }
    assert.equal(puts, 0);
    await name.fill("Rain Radio");
    await page.getByLabel("Description", { exact: true }).fill("<b>Calm music</b>\nFor our server.");
    assert.equal(await page.locator(".discord-profile #profile-nick").inputValue(), "Rain Radio");
    assert.equal(await page.locator(".discord-profile #profile-bio").inputValue(), "<b>Calm music</b>\nFor our server.");
    assert.equal(await page.locator(".discord-profile b").count(), 0);
    assert.equal(await page.locator(".profile-layout").count(), 0, "No separate editor/preview columns");
    for (const key of ["avatar", "banner"]) {
      const trigger = page.getByRole("button", { name: `Edit ${key}`, exact: true });
      await trigger.focus(); assert.equal(await trigger.evaluate(e => e === document.activeElement), true);
      const chooser = page.waitForEvent("filechooser");
      await trigger.press("Enter");
      const fileChooser = await chooser;
      assert.equal(fileChooser.isMultiple(), false);
      await fileChooser.setFiles({ name: "sample.png", mimeType: "image/png", buffer: picture });
      await page.waitForFunction(key => document.querySelector(`.profile-${key}`).src.startsWith("data:"), key);
    }
    assert.equal(puts, 0, "Previews must not upload before Apply");
    await page.getByRole("button", { name: "Apply profile", exact: true }).click();
    await page.getByText("Discord did not apply the profile. Try again.", { exact: true }).waitFor();
    assert.equal(await name.inputValue(), "Rain Radio");
    assert.match(await page.locator(".profile-avatar").getAttribute("src"), /^data:/);
    assert.equal(await page.getByText("Discord applied the profile to this server.", { exact: true }).count(), 0);
    fail = false;
    await page.getByRole("button", { name: "Apply profile", exact: true }).click();
    await page.getByText("Discord applied the profile to this server.", { exact: true }).waitFor();
    await page.waitForFunction(() => document.getElementById("profile-nick")?.disabled === false);
    await name.fill("Another draft");
    assert.equal(await page.getByText("Discord applied the profile to this server.", { exact: true }).isVisible(), false);
    await page.reload(); await page.waitForFunction(() => document.getElementById("profile-nick")?.disabled === false);
    assert.equal(await name.inputValue(), "Rain Radio");
    await page.locator("#profile-avatar").setInputFiles({ name: "large.png", mimeType: "image/png", buffer: Buffer.alloc(2 * 1024 * 1024 + 1) });
    await page.getByText("Images must be no larger than 2 MB.", { exact: true }).waitFor();
    assert.equal(await page.getByRole("button", { name: "Apply profile", exact: true }).isDisabled(), true);
    await page.getByRole("button", { name: "Undo avatar selection", exact: true }).click();
    assert.equal(await page.getByRole("button", { name: "Apply profile", exact: true }).isEnabled(), true);
    const overflow = await page.evaluate(() => ({ width: document.documentElement.scrollWidth, viewport: innerWidth, elements: [...document.querySelectorAll("#profile-form *")].filter(e => e.getBoundingClientRect().right > innerWidth).map(e => e.id || e.className) }));
    if(overflow.width > overflow.viewport) await page.screenshot({path:"output/profile-overflow-state.png",fullPage:true});
    assert.equal(overflow.width > overflow.viewport, false, `profile overflow ${width}: ${JSON.stringify(overflow)}`);
    if ([390, 1440].includes(width)) {
      await page.evaluate(() => { window.scrollTo(0, 0); document.getElementById("main").scrollTop = 0; });
      await page.screenshot({ path: `output/music-profile-${width}.png`, fullPage: true });
    }
    await page.getByRole("button", { name: "Restore Vozen profile", exact: true }).click();
    await page.waitForFunction(() => document.getElementById("profile-nick").value === "");
    assert.equal(await name.getAttribute("placeholder"), "Vozen Music");
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log("Profile UI: seven widths, live text/images, blocked names, no early uploads, failures, retry, reload, reset and size validation passed.");
} finally { await browser.close(); }
