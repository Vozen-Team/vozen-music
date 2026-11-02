import assert from "node:assert/strict";
import fs from "node:fs";
const { chromium } = await import(
  process.env.VOZEN_PLAYWRIGHT_MODULE || "playwright"
);
const browser = await chromium.launch();
const errors = [];
fs.mkdirSync("output", { recursive: true });
try {
  for (const width of [320, 360, 390, 430, 768, 900, 1440]) {
    const context = await browser.newContext({
        viewport: { width, height: 900 },
      }),
      page = await context.newPage();
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto("http://localhost:4783/music/dashboard");
    await page.getByRole("heading", { name: "Your servers.", exact: true }).waitFor();
    assert.equal(await page.locator("#sidebar").isVisible(), false);
    assert.equal(await page.locator("#editor").isVisible(), false);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `picker overflow ${width}`);
    await page.locator(".server-choice").first().click();
    await page
      .getByRole("heading", { name: "Overview", exact: true })
      .waitFor();
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
      "Playback",
      "Access & Channels",
      "Stations",
      "Community",
      "Bot Profile",
      "Quick Setup",
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
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    assert.match(await page.locator(".setup-progress").innerText(), /Step 4/);
    const level = String(
      30 + [320, 360, 390, 430, 768, 900, 1440].indexOf(width),
    );
    await go("Playback");
    await page
      .getByLabel("Default and current volume", { exact: true })
      .fill(level);
    await page.locator("#music-save").click();
    await page
      .getByText("Settings saved for this server.", { exact: true })
      .waitFor();
    await page.reload();
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
    await go("Access & Channels");
    await page.getByRole("button", { name: "← Choose another server", exact: true }).click();
    await page.getByRole("heading", { name: "Your servers.", exact: true }).waitFor();
    assert.equal(new URL(page.url()).searchParams.has("server"), false);
    await page.locator(".server-choice").first().focus();
    await page.keyboard.press("Enter");
    await go("Access & Channels");
    const rule = page.getByLabel("List behaviour").first();
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
    "Music dashboard: seven pages, seven widths, Quick Setup, save/reload, discard, menus and JavaScript checks passed.",
  );
} finally {
  await browser.close();
}
