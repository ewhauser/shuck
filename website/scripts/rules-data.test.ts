import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import generatedRulesJson from "../app/lib/rules-data.generated.json";
import {
  generateRulesData,
  parseImplementedRulesFromRegistry,
  rulesDataSourcePaths,
} from "./rules-data";

const __dirname = dirname(fileURLToPath(import.meta.url));

function collectWebsiteSourceFiles(dir: string): string[] {
  const files: string[] = [];

  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const stats = statSync(path);

    if (stats.isDirectory()) {
      files.push(...collectWebsiteSourceFiles(path));
    } else if (/\.(?:mdx|tsx?)$/u.test(entry)) {
      files.push(path);
    }
  }

  return files;
}

test("parseImplementedRulesFromRegistry handles multiline registry tuples", () => {
  const implementedRules = parseImplementedRulesFromRegistry(`
declare_rules! {
    (
        "C085",
        Category::Correctness,
        Severity::Warning,
        StderrBeforeStdoutRedirect
    ),
    ("S001", Category::Style, Severity::Warning, UnquotedExpansion),
}
`);

  assert.equal(implementedRules.get("C085"), "Warning");
  assert.equal(implementedRules.get("S001"), "Warning");
});

test("generated rules data matches registry implementation state", () => {
  const generatedRules = generatedRulesJson as ReturnType<typeof generateRulesData>;
  const expectedRules = generateRulesData();
  const registryRules = parseImplementedRulesFromRegistry(
    readFileSync(rulesDataSourcePaths.registryPath, "utf-8"),
  );

  assert.ok(registryRules.size > 250);
  assert.deepEqual(generatedRules, expectedRules);
});

test("website diagnostic examples use current rule codes and messages", () => {
  const knownExamples = [
    {
      code: "S001",
      level: "warning",
      message: "quote parameter expansions to avoid word splitting and globbing",
    },
    {
      code: "S014",
      level: "warning",
      message: "quote star-splat expansions to preserve argument boundaries",
    },
    {
      code: "S005",
      level: "warning",
      message: "prefer `$(...)` over legacy backtick substitution",
    },
    {
      code: "C001",
      level: "warning",
      message: "variable `summary` is assigned but never used",
    },
  ];
  const source = [
    ...collectWebsiteSourceFiles(join(__dirname, "../app")),
    ...collectWebsiteSourceFiles(join(__dirname, "../content")),
  ]
    .map((path) => readFileSync(path, "utf-8"))
    .join("\n");
  const documentedDiagnostics = [
    ...source.matchAll(/\b(warning|error)\[([A-Z]\d{3})\]/g),
  ].map((match) => `${match[1]}[${match[2]}]`);
  const expectedDiagnostics = knownExamples.map(
    (example) => `${example.level}[${example.code}]`,
  );

  const generatedByCode = new Map(
    (generatedRulesJson as Array<{ code: string; implemented: boolean; severity: string | null }>)
      .map((rule) => [rule.code, rule]),
  );
  const documentedRuleCodes = [
    ...source.matchAll(/\b([CSXPK]\d{3})\b/g),
  ].map((match) => match[1]);

  for (const code of new Set(documentedRuleCodes)) {
    assert.ok(generatedByCode.has(code), `${code} should exist in generated rule data`);
  }

  assert.deepEqual(documentedDiagnostics.sort(), expectedDiagnostics.sort());

  for (const example of knownExamples) {
    const rule = generatedByCode.get(example.code);
    assert.equal(rule?.implemented, true, `${example.code} should be implemented`);
    assert.equal(
      rule?.severity?.toLowerCase(),
      example.level,
      `${example.code} should keep the documented severity`,
    );
    assert.match(source, new RegExp(example.message.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }
});
