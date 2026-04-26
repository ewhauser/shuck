import { Suspense } from "react";
import type { Metadata } from "next";
import { allRules, categories } from "@/app/lib/rules-data";
import type { RuleListItem } from "@/app/lib/rules-data";
import RulesTable from "@/app/components/docs/RulesTable";
import { RuleStatusDot } from "@/app/components/docs/RuleBadge";

export const metadata: Metadata = {
  title: "Rules",
  description: "Browse all shuck lint rules for shell scripts.",
};

export default function RulesIndexPage() {
  const fullyImplemented = allRules.filter((r) => r.status === "implemented").length;
  const implementedWithKnownShellcheckDivergences = allRules.filter(
    (r) => r.status === "implemented_with_known_shellcheck_divergences",
  ).length;
  const planned = allRules.length - fullyImplemented - implementedWithKnownShellcheckDivergences;
  const implementedFixes = allRules.filter((r) => r.fixStatus === "implemented").length;
  const plannedFixes = allRules.filter((r) => r.fixStatus === "planned").length;

  // Strip heavy fields (rationale, examples, etc.) before sending to the client component.
  const listRules: RuleListItem[] = allRules.map(
    ({
      code,
      shellcheckCode,
      name,
      category,
      description,
      implemented,
      status,
      fixStatus,
      fixSafety,
    }) => ({
      code,
      shellcheckCode,
      name,
      category,
      description,
      implemented,
      status,
      fixStatus,
      fixSafety,
    }),
  );

  return (
    <div className="max-w-5xl">
      <h1 className="mb-2 text-2xl font-bold text-fg-primary">Rules</h1>
      <p className="mb-6 text-fg-secondary">
        {allRules.length} rules across {categories.length} categories.{" "}
        <span className="text-green-400">{fullyImplemented} implemented</span>,{" "}
        <span className="text-yellow-300">
          {implementedWithKnownShellcheckDivergences} with known ShellCheck divergences
        </span>
        , {planned} planned.
        {" "}{implementedFixes} autofixes implemented, {plannedFixes} planned.
      </p>
      <div className="mb-6 flex flex-wrap gap-x-4 gap-y-2 text-sm text-fg-secondary">
        <span className="inline-flex items-center gap-2">
          <RuleStatusDot status="implemented" />
          Implemented
        </span>
        <span className="inline-flex items-center gap-2">
          <RuleStatusDot status="implemented_with_known_shellcheck_divergences" />
          Implemented, with known ShellCheck divergences in corpus metadata
        </span>
        <span className="inline-flex items-center gap-2">
          <RuleStatusDot status="planned" />
          Planned
        </span>
      </div>
      <Suspense>
        <RulesTable rules={listRules} />
      </Suspense>
    </div>
  );
}
