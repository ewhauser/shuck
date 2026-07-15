import assert from "node:assert/strict";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageDirectory = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
  process.argv[2] ?? "target/wasm-test/shuck-wasm",
);
const require = createRequire(import.meta.url);
const shuck = require(packageDirectory);
const packageJson = require(path.join(packageDirectory, "package.json"));

const diagnostics = shuck.lint("echo $name\n", {
  filename: "script.bash",
  select: ["ALL"],
});

assert.ok(Array.isArray(diagnostics));
assert.ok(diagnostics.some((diagnostic) => diagnostic.code === "S001"));
assert.ok(Array.isArray(shuck.lint("echo ok\n")));
assert.equal(shuck.version(), packageJson.version);
assert.equal(
  shuck.format("hello(){\necho hi\n}\n", {
    shell: "bash",
    indentStyle: "space",
    indentWidth: 2,
  }),
  "hello() {\n  echo hi\n}\n",
);

console.log(`smoke-tested ${packageJson.name}@${packageJson.version}`);
