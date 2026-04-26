import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
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
  safe_fix?: boolean;
  fix_description?: string | null;
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

export type FixAvailability = "none" | "sometimes" | "always";
export type FixStatus = "none" | "planned" | "implemented";
export type FixSafety = "safe" | "unsafe";

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
  fixStatus: FixStatus;
  fixAvailability: FixAvailability;
  fixSafety: FixSafety | null;
  fixDescription: string | null;
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
  linterSrcDir: join(repoRootDir, "crates/shuck-linter/src"),
};

export const rulesDataOutputPath = join(
  repoRootDir,
  "website/app/lib/rules-data.generated.json",
);

interface ImplementedRule {
  severity: string;
  violationName: string;
}

export function parseImplementedRulesFromRegistry(
  registrySource: string,
): Map<string, ImplementedRule> {
  const implementedRules = new Map<string, ImplementedRule>();
  const rulePattern =
    /\(\s*"([A-Z]\d+)",\s*Category::\w+,\s*Severity::(\w+),\s*(\w+)\s*\)/g;
  let match: RegExpExecArray | null;

  while ((match = rulePattern.exec(registrySource)) !== null) {
    implementedRules.set(match[1], {
      severity: match[2],
      violationName: match[3],
    });
  }

  return implementedRules;
}

function collectRustSourceFiles(dir: string): string[] {
  const files: string[] = [];

  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const stats = statSync(path);
    if (stats.isDirectory()) {
      files.push(...collectRustSourceFiles(path));
    } else if (path.endsWith(".rs")) {
      files.push(path);
    }
  }

  return files;
}

export function parseImplementedFixAvailability(
  source: string,
): Map<string, FixAvailability> {
  const fixAvailability = new Map<string, FixAvailability>();
  const violationPattern =
    /impl\s+Violation\s+for\s+(\w+)\s*\{(?:(?!impl\s+Violation\s+for)[\s\S])*?const\s+FIX_AVAILABILITY:\s*FixAvailability\s*=\s*FixAvailability::(None|Sometimes|Always)\s*;/g;
  let match: RegExpExecArray | null;

  while ((match = violationPattern.exec(source)) !== null) {
    fixAvailability.set(
      match[1],
      match[2].toLowerCase() as FixAvailability,
    );
  }

  return fixAvailability;
}

export function collectImplementedFixAvailability(
  linterSrcDir: string,
): Map<string, FixAvailability> {
  const source = collectRustSourceFiles(linterSrcDir)
    .map((path) => readFileSync(path, "utf-8"))
    .join("\n");
  return parseImplementedFixAvailability(source);
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
  implementedRules: Map<string, ImplementedRule>;
  implementedFixAvailability: Map<string, FixAvailability>;
  rulesWithKnownShellcheckDivergences: Set<string>;
}): RuleData[] {
  const yamlFiles = readdirSync(input.rulesDir)
    .filter((file) => file.endsWith(".yaml"))
    .sort();

  const rules = yamlFiles.map((file) => {
    const content = readFileSync(join(input.rulesDir, file), "utf-8");
    const yaml = parseYaml(content) as RuleYaml;
    const code = yaml.new_code;
    const implementedRule = input.implementedRules.get(code);
    const implemented = implementedRule !== undefined;
    const hasKnownShellcheckDivergences =
      input.rulesWithKnownShellcheckDivergences.has(code);
    const severity = implementedRule?.severity ?? null;
    const status: RuleStatus = !implemented
      ? "planned"
      : hasKnownShellcheckDivergences
        ? "implemented_with_known_shellcheck_divergences"
        : "implemented";
    const implementedFixAvailability = implementedRule
      ? input.implementedFixAvailability.get(implementedRule.violationName) ?? "none"
      : "none";
    const fixDescription = yaml.fix_description ?? null;
    const fixStatus: FixStatus =
      implementedFixAvailability !== "none"
        ? "implemented"
        : fixDescription
          ? "planned"
          : "none";
    const fixSafety: FixSafety | null = fixDescription
      ? yaml.safe_fix
        ? "safe"
        : "unsafe"
      : null;

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
      fixStatus,
      fixAvailability: implementedFixAvailability,
      fixSafety,
      fixDescription,
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
    implementedFixAvailability:
      collectImplementedFixAvailability(join(rootDir, "crates/shuck-linter/src")),
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
