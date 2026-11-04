import assert from "node:assert/strict";
import fs from "node:fs";
const { chromium } = await import(
  process.env.VOZEN_PLAYWRIGHT_MODULE || "playwright"
);
const browser = await chromium.launch();
const dashboard = (process.env.VOZEN_PREVIEW_URL || "http://localhost:4783") + "/music/dashboard";
const errors = [];
fs.mkdirSync("output", { recursive: true });
try {
  for (const width of [320, 360, 390, 430, 768, 900, 1440]) {
    const context = await browser.newContext({
        viewport: { width, height: 900 },
      }),
      page = await context.newPage();
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto(dashboard);
    await page.getByRole("heading", { name: "Your servers.", exact: true }).waitFor();
    assert.equal(await page.locator("#sidebar").isVisible(), false);
    assert.equal(await page.locator("#editor").isVisible(), false);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `picker overflow ${width}`);
    await page.locator(".server-choice").first().click();
    await page
      .getByRole("heading", { name: "Overview", exact: true })
      .waitFor();
    assert.equal(await page.getByRole("button", { name: "Quick Setup", exact: true, includeHidden: true }).count(), 0);
    assert.equal(await page.getByRole("heading", { name: "Your choices", exact: true }).count(), 1);
    await page.locator(".overview-profile .profile-preview-name").getByText("Vozen Music", { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overview overflow ${width}`);
    await page.getByRole("link", { name: "Open Bot Profile", exact: true }).click();
    await page.getByRole("heading", { name: "Bot Profile", exact: true }).waitFor();
    assert.equal(new URL(page.url()).searchParams.get("page"), "profile");
    if (width === 390) {
      await page.goto(dashboard + "?server=1&page=setup");
      await page.getByRole("heading", { name: "Overview", exact: true }).waitFor();
      assert.equal(new URL(page.url()).pathname, "/music/1");
      assert.equal(new URL(page.url()).search, "");
    }
    if (width === 390)
      console.log(
        "Mobile heading position:",
        await page
          .getByRole("heading", { name: "Overview", exact: true })
          .evaluate((e) => e.getBoundingClientRect().top),
      );
    const go = async (name) => {
      const b = page
        .locator("#music-pages")
        .getByRole("button", { name, exact: true });
      if (!(await b.isVisible()))
        await page.locator("#sections-trigger").click();
      await b.click();
    };
    for (const name of [
      "Settings",
      "Stations",
      "Bot Profile",
    ]) {
      await go(name);
      await page.getByRole("heading", { name, exact: true }).waitFor();
      assert.equal(
        await page.evaluate(
          () => document.documentElement.scrollWidth > innerWidth,
        ),
        false,
        `overflow ${name} ${width}`,
      );
    }
    const level = String(
      30 + [320, 360, 390, 430, 768, 900, 1440].indexOf(width),
    );
    await go("Settings");
    const visibleSection = page.locator(".settings-section:visible > h2");
    const activeSection = page.locator(".settings-jump [aria-current=page]");
    assert.deepEqual(await visibleSection.allTextContents(), ["Playback"]);
    assert.equal(await activeSection.textContent(), "Playback");
    assert.equal(await page.locator("#music-pages button").count(), 4);
    await page
      .getByLabel("Default and current volume", { exact: true })
      .fill(level);
    if (width === 390) {
      const volume = page.getByLabel("Default and current volume", { exact: true });
      await volume.fill("101");
      await page.getByRole("link", { name: "Access & Channels", exact: true }).click();
      await page.locator("#music-save").click();
      assert.deepEqual(await visibleSection.allTextContents(), ["Playback"]);
      assert.equal(await volume.evaluate(input => input === document.activeElement && !input.validity.valid), true);
      await volume.fill(level);
    }
    await page.getByRole("link", { name: "Community", exact: true }).press("Enter");
    assert.deepEqual(await visibleSection.allTextContents(), ["Community"]);
    assert.equal(await activeSection.textContent(), "Community");
    await page.locator("#music-save").click();
    await page
      .getByText("Settings saved for this server.", { exact: true })
      .waitFor();
    await page.reload();
    await page.getByRole("heading", { name: "Community", exact: true }).waitFor();
    assert.deepEqual(await visibleSection.allTextContents(), ["Community"]);
    if (width === 390) {
      const control = name => page.getByRole("button", { name, exact: true });
      await control("Turn on tracking").click();
      await page.getByText("Hidden from the leaderboard", { exact: true }).waitFor();
      await control("Show my entry").click();
      await control("Hide my entry").waitFor();
      await control("Hide my entry").click();
      await control("Show my entry").waitFor();
      await page.getByText("Delete your data", { exact: true }).click();
      page.once("dialog", dialog => dialog.dismiss());
      await control("Erase my recorded time").click();
      assert.equal(await control("Turn off tracking").isVisible(), true);
      page.once("dialog", dialog => dialog.accept());
      await control("Erase my recorded time").click();
      await control("Turn on tracking").waitFor();
      assert.equal(await control("Show my entry").count(), 0);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    }
    await page.getByRole("link", { name: "Playback", exact: true }).click();
    await page
      .getByLabel("Default and current volume", { exact: true })
      .waitFor();
    assert.equal(
      await page
        .getByLabel("Default and current volume", { exact: true })
        .inputValue(),
      level,
    );
    await page
      .getByLabel("Default and current volume", { exact: true })
      .fill("55");
    await page.locator("#music-discard").click();
    assert.equal(
      await page
        .getByLabel("Default and current volume", { exact: true })
        .inputValue(),
      level,
    );
    await page.getByRole("link", { name: "Access & Channels", exact: true }).click();
    assert.equal(new URL(page.url()).searchParams.get("section"), "access");
    assert.deepEqual(await visibleSection.allTextContents(), ["Access & Channels"]);
    await page.locator("#server-select").selectOption("__choose_server__");
    await page.getByRole("heading", { name: "Your servers.", exact: true }).waitFor();
    assert.equal(new URL(page.url()).searchParams.has("server"), false);
    await page.locator(".server-choice").first().focus();
    await page.keyboard.press("Enter");
    await page.getByRole("link", { name: "Edit Access & Channels", exact: true }).click();
    assert.equal(await page.locator("#settings-access").evaluate(e => e === document.activeElement), true);
    const restrictions = page.locator(".settings-channel-rules summary");
    const rule = page.getByLabel("List behaviour").first();
    assert.equal(await rule.isVisible(), false);
    await restrictions.press("Enter");
    assert.equal(await rule.isVisible(), true);
    await rule.selectOption(
      (await rule.inputValue()) === "true" ? "false" : "true",
    );
    await page.locator("#music-save").click();
    await page
      .getByText("Settings saved for this server.", { exact: true })
      .waitFor();
    assert.equal(
      await page.evaluate(
        () =>
          typeof JSON.parse(localStorage.getItem("music-draft:1") || "{}")
            .voice_rules?.allow,
      ),
      "undefined",
    );
    await page.evaluate(() => window.scrollTo(0, 0));
    if (width === 390 || width === 1440)
      await page.screenshot({
        path: `output/music-expanded-${width}.png`,
        fullPage: true,
      });
    if (width < 901) {
      await page.locator("#sections-trigger").click();
      await page.locator("#sections-trigger").press("Escape");
      assert.equal(
        await page.locator("#sections-trigger").getAttribute("aria-expanded"),
        "false",
      );
    }
    await context.close();
  }
  assert.deepEqual(errors, []);
  console.log(
    "Music dashboard: four pages, seven widths, Settings sections, save/reload, discard and menus passed.",
  );
} finally {
  await browser.close();
}
