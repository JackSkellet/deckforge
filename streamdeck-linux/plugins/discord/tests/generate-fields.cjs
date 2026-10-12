// Classic local script avoids fetch(file://...) in the native WebKit inspector.
const fs = require("node:fs"),
  path = require("node:path");
const root = path.join(__dirname, "../ui");
const expected =
  "// Generated from fields.json by tests/generate-fields.cjs.\nwindow.DeckforgeDiscordFields = " +
  JSON.stringify(
    JSON.parse(fs.readFileSync(path.join(root, "fields.json"), "utf8")),
    null,
    2,
  ) +
  ";\n";
const target = path.join(root, "fields.js");
if (process.argv.includes("--check")) {
  if (!fs.existsSync(target) || fs.readFileSync(target, "utf8") !== expected)
    throw Error(
      "Run node plugins/discord/tests/generate-fields.cjs to refresh fields.js",
    );
} else fs.writeFileSync(target, expected);
