import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseYaml } from "yaml";

const __dirname = dirname(fileURLToPath(import.meta.url));

interface RuleYaml {
  legacy_name: string;
  new_category: string;
  new_code: string;
  shellcheck_code?: string;
  shells?: string[];
  description: string;
  rationale: string;
  examples?: Array<{ kind: string; code?: string }>;
}

interface CorpusMetadataYaml {
  review_all_divergences?: boolean;
  reviewed_divergences?: unknown[];
}

export type RuleStatus =
  | "planned"
  | "implemented"
  | "implemented_with_known_shellcheck_divergences";

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
  status: RuleStatus;
  hasKnownShellcheckDivergences: boolean;
  severity: string | null;
  examples: Array<{ kind: string; code: string }>;
}

export const CATEGORY_PREFIX: Record<string, string> = {
  Correctness: "C",
  Style: "S",
  Portability: "X",
  Performance: "P",
  Security: "K",
};

const repoRootDir = resolve(__dirname, "../..");

export const rulesDataSourcePaths = {
  rulesDir: join(repoRootDir, "docs/rules"),
  corpusMetadataDir: join(
    repoRootDir,
    "crates/shuck-cli/tests/testdata/corpus-metadata",
  ),
  registryPath: join(repoRootDir, "crates/shuck-linter/src/registry.rs"),
};

export const rulesDataOutputPath = join(
  repoRootDir,
  "website/app/lib/rules-data.generated.json",
);

export function parseImplementedRulesFromRegistry(
  registrySource: string,
): Map<string, string> {
  const implementedRules = new Map<string, string>();
  const rulePattern = /\(\s*"([A-Z]\d+)",\s*Category::\w+,\s*Severity::(\w+),/g;
  let match: RegExpExecArray | null;

  while ((match = rulePattern.exec(registrySource)) !== null) {
    implementedRules.set(match[1], match[2]);
  }

  return implementedRules;
}

export function collectRulesWithKnownShellcheckDivergences(
  corpusMetadataDir: string,
): Set<string> {
  const rules = new Set<string>();

  for (const file of readdirSync(corpusMetadataDir)) {
    if (!file.endsWith(".yaml")) {
      continue;
    }

    const content = readFileSync(join(corpusMetadataDir, file), "utf-8");
    const yaml = parseYaml(content) as CorpusMetadataYaml | null;
    const reviewedDivergences = Array.isArray(yaml?.reviewed_divergences)
      ? yaml.reviewed_divergences
      : [];

    if (yaml?.review_all_divergences || reviewedDivergences.length > 0) {
      rules.add(file.replace(/\.yaml$/u, "").toUpperCase());
    }
  }

  return rules;
}

export function buildRulesData(input: {
  rulesDir: string;
  implementedRules: Map<string, string>;
  rulesWithKnownShellcheckDivergences: Set<string>;
}): RuleData[] {
  const yamlFiles = readdirSync(input.rulesDir)
    .filter((file) => file.endsWith(".yaml"))
    .sort();

  const rules = yamlFiles.map((file) => {
    const content = readFileSync(join(input.rulesDir, file), "utf-8");
    const yaml = parseYaml(content) as RuleYaml;
    const code = yaml.new_code;
    const implemented = input.implementedRules.has(code);
    const hasKnownShellcheckDivergences =
      input.rulesWithKnownShellcheckDivergences.has(code);
    const severity = input.implementedRules.get(code) ?? null;
    const status: RuleStatus = !implemented
      ? "planned"
      : hasKnownShellcheckDivergences
        ? "implemented_with_known_shellcheck_divergences"
        : "implemented";

    return {
      code,
      name: yaml.legacy_name,
      category: yaml.new_category,
      categoryPrefix: CATEGORY_PREFIX[yaml.new_category] ?? "?",
      description: yaml.description,
      rationale: yaml.rationale,
      shells: yaml.shells ?? [],
      shellcheckCode: yaml.shellcheck_code ?? null,
      implemented,
      status,
      hasKnownShellcheckDivergences,
      severity,
      examples: (yaml.examples ?? []).map((example) => ({
        kind: example.kind,
        code: example.code?.trimEnd() ?? "",
      })),
    };
  });

  rules.sort((left, right) => {
    const prefixOrder = "CSXPK";
    const prefixDelta =
      prefixOrder.indexOf(left.categoryPrefix) -
      prefixOrder.indexOf(right.categoryPrefix);

    if (prefixDelta !== 0) {
      return prefixDelta;
    }

    return Number.parseInt(left.code.slice(1), 10) -
      Number.parseInt(right.code.slice(1), 10);
  });

  return rules;
}

export function generateRulesData(rootDir: string = repoRootDir): RuleData[] {
  const rulesDir = join(rootDir, "docs/rules");
  const corpusMetadataDir = join(
    rootDir,
    "crates/shuck-cli/tests/testdata/corpus-metadata",
  );
  const registryPath = join(rootDir, "crates/shuck-linter/src/registry.rs");

  return buildRulesData({
    rulesDir,
    implementedRules: parseImplementedRulesFromRegistry(
      readFileSync(registryPath, "utf-8"),
    ),
    rulesWithKnownShellcheckDivergences:
      collectRulesWithKnownShellcheckDivergences(corpusMetadataDir),
  });
}

export function writeRulesData(
  outputPath: string = rulesDataOutputPath,
): RuleData[] {
  const rules = generateRulesData();
  writeFileSync(outputPath, JSON.stringify(rules, null, 2) + "\n");
  return rules;
}
