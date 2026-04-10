import rulesJson from "./rules-data.generated.json";

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
