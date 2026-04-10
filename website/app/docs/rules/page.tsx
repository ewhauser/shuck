import { Suspense } from "react";
import type { Metadata } from "next";
import { allRules, categories } from "@/app/lib/rules-data";
import RulesTable from "@/app/components/docs/RulesTable";

export const metadata: Metadata = {
  title: "Rules",
  description: "Browse all shuck lint rules for shell scripts.",
};

export default function RulesIndexPage() {
  const implemented = allRules.filter((r) => r.implemented).length;

  return (
    <div className="max-w-5xl">
      <h1 className="mb-2 text-2xl font-bold text-fg-primary">Rules</h1>
      <p className="mb-6 text-fg-secondary">
        {allRules.length} rules across {categories.length} categories.{" "}
        <span className="text-green-400">{implemented} implemented</span>,{" "}
        {allRules.length - implemented} planned.
      </p>
      <Suspense>
        <RulesTable rules={allRules} />
      </Suspense>
    </div>
  );
}
