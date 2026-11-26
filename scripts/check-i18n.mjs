// Checks that every dashboard language translates the same keys, and that
// every literal t("…") key used by the dashboard exists in each language.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const web = path.join(import.meta.dirname, "..", "web");
const source = fs.readFileSync(path.join(web, "dashboard-i18n.js"), "utf8").replaceAll("\r\n", "\n");
const languages = {};
for (const match of source.matchAll(/^ {2}"([a-z]{2})": \{\n([\s\S]*?)\n {2}\}/gm))
  languages[match[1]] = JSON.parse(`{${match[2]}}`);
const names = Object.keys(languages);
assert.ok(names.length >= 9, `found ${names.length} language blocks`);

const reference = new Set(Object.keys(languages[names[0]]));
for (const name of names) {
  const keys = new Set(Object.keys(languages[name]));
  const missing = [...reference].filter((k) => !keys.has(k));
  const extra = [...keys].filter((k) => !reference.has(k));
  assert.deepEqual({ name, missing, extra }, { name, missing: [], extra: [] });
}

const script = fs.readFileSync(path.join(web, "dashboard.js"), "utf8");
const used = new Set([...script.matchAll(/\bt\("((?:[^"\\]|\\.)*)"/g)].map((m) => JSON.parse(`"${m[1]}"`)));
// Keys made only of placeholders and punctuation read the same in every language.
const wordy = (key) => /\p{L}/u.test(key.replace(/\{[^}]+\}/g, ""));
const untranslated = [...used].filter((key) => wordy(key) && !reference.has(key));
assert.deepEqual(untranslated, [], "dashboard keys without translations");
console.log(`Dashboard i18n: ${names.length} languages share ${reference.size} keys; all ${used.size} literal keys are translated.`);
