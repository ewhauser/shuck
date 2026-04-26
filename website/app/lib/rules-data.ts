import rulesJson from "./rules-data.generated.json";

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

/** Lightweight subset for the rules list table (avoids shipping rationale/examples to client). */
export interface RuleListItem {
  code: string;
  shellcheckCode: string | null;
  name: string;
  category: string;
  description: string;
  implemented: boolean;
  status: RuleStatus;
  fixStatus: FixStatus;
  fixSafety: FixSafety | null;
}

export const allRules: RuleData[] = rulesJson as RuleData[];

export function getRuleByCode(code: string): RuleData | undefined {
  return allRules.find((r) => r.code.toLowerCase() === code.toLowerCase());
}

export const categories = [
  { name: "Correctness", prefix: "C" },
  { name: "Style", prefix: "S" },
  { name: "Portability", prefix: "X" },
  { name: "Performance", prefix: "P" },
  { name: "Security", prefix: "K" },
] as const;
