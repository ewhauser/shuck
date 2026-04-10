import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseYaml } from "yaml";

const __dirname = dirname(fileURLToPath(import.meta.url));

interface RuleYaml {
  legacy_name: string;
  new_category: string;
  new_code: string;
  runtime_kind: string;
  shellcheck_code: string;
  shells: string[];
  description: string;
  rationale: string;
  examples?: Array<{ kind: string; code: string }>;
}

export interface RuleData {
  code: string;
  name: string;
  category: string;
  categoryPrefix: string;
  description: string;
  rationale: string;
  shells: string[];
  shellcheckCode: string | null;
  implemented: boolean;
  severity: string | null;
  examples: Array<{ kind: string; code: string }>;
}

const CATEGORY_PREFIX: Record<string, string> = {
  Correctness: "C",
  Style: "S",
  Portability: "X",
  Performance: "P",
  Security: "K",
};

const rootDir = resolve(__dirname, "../..");
const rulesDir = join(rootDir, "docs/rules");
const registryPath = join(rootDir, "crates/shuck-linter/src/registry.rs");
const outPath = join(__dirname, "../app/lib/rules-data.generated.json");

// Parse registry.rs for implemented rules and their severities
const registrySource = readFileSync(registryPath, "utf-8");
const implementedRules = new Map<string, string>();

// Match tuples like: ("C001", Category::Correctness, Severity::Warning, UnusedAssignment),
// Also handles multiline formatting where the tuple is split across lines.
const rulePattern = /\(\s*"([A-Z]\d+)",\s*Category::\w+,\s*Severity::(\w+),/g;
let match: RegExpExecArray | null;
while ((match = rulePattern.exec(registrySource)) !== null) {
  implementedRules.set(match[1], match[2]);
}

// Read and parse all YAML rule files
const yamlFiles = readdirSync(rulesDir)
  .filter((f) => f.endsWith(".yaml") && f !== "validate.sh")
  .sort();

const rules: RuleData[] = yamlFiles.map((file) => {
  const content = readFileSync(join(rulesDir, file), "utf-8");
  const yaml = parseYaml(content) as RuleYaml;
  const code = yaml.new_code;
  const severity = implementedRules.get(code) ?? null;

  return {
    code,
    name: yaml.legacy_name,
    category: yaml.new_category,
    categoryPrefix: CATEGORY_PREFIX[yaml.new_category] ?? "?",
    description: yaml.description,
    rationale: yaml.rationale,
    shells: yaml.shells ?? [],
    shellcheckCode: yaml.shellcheck_code ?? null,
    implemented: implementedRules.has(code),
    severity,
    examples: (yaml.examples ?? []).map((ex) => ({
      kind: ex.kind,
      code: ex.code?.trimEnd() ?? "",
    })),
  };
});

// Sort: by category prefix, then numeric code
rules.sort((a, b) => {
  const prefixOrder = "CSXPK";
  const pi = prefixOrder.indexOf(a.categoryPrefix) - prefixOrder.indexOf(b.categoryPrefix);
  if (pi !== 0) return pi;
  const aNum = parseInt(a.code.slice(1));
  const bNum = parseInt(b.code.slice(1));
  return aNum - bNum;
});

writeFileSync(outPath, JSON.stringify(rules, null, 2) + "\n");

console.log(
  `Generated ${rules.length} rules (${implementedRules.size} implemented) -> ${outPath}`,
);
