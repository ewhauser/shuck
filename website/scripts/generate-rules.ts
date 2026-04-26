import { rulesDataOutputPath, writeRulesData } from "./rules-data";

const rules = writeRulesData();
const implemented = rules.filter((rule) => rule.implemented).length;
const implementedWithKnownShellcheckDivergences = rules.filter(
  (rule) => rule.status === "implemented_with_known_shellcheck_divergences",
).length;

console.log(
  `Generated ${rules.length} rules (${implemented} implemented, ${implementedWithKnownShellcheckDivergences} with known ShellCheck divergences) -> ${rulesDataOutputPath}`,
);
