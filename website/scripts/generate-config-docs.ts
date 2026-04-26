import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const rootDir = resolve(__dirname, "../..");
const outPath = join(__dirname, "../content/settings/index.mdx");

const content = execFileSync(
  "cargo",
  [
    "run",
    "--quiet",
    "--manifest-path",
    join(rootDir, "Cargo.toml"),
    "--package",
    "shuck-cli",
    "--example",
    "generate_config_docs",
  ],
  {
    cwd: rootDir,
    encoding: "utf-8",
  },
);

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, content);

console.log(`Generated configuration settings reference -> ${outPath}`);
