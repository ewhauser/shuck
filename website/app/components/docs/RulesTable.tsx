"use client";

import { useState, useMemo } from "react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Search } from "lucide-react";
import type { RuleData } from "@/app/lib/rules-data";
import { categories } from "@/app/lib/rules-data";
import { CategoryBadge } from "./RuleBadge";

interface Props {
  rules: RuleData[];
}

export default function RulesTable({ rules }: Props) {
  const searchParams = useSearchParams();
  const initialCategory = searchParams.get("category") ?? "All";

  const [search, setSearch] = useState("");
  const [selectedCategory, setSelectedCategory] = useState(initialCategory);

  const filtered = useMemo(() => {
    let result = rules;
    if (selectedCategory !== "All") {
      result = result.filter((r) => r.category === selectedCategory);
    }
    if (search) {
      const q = search.toLowerCase();
      result = result.filter(
        (r) =>
          r.code.toLowerCase().includes(q) ||
          r.name.toLowerCase().includes(q) ||
          r.description.toLowerCase().includes(q),
      );
    }
    return result;
  }, [rules, search, selectedCategory]);

  // Group by category for display
  const grouped = useMemo(() => {
    const groups: Array<{ category: string; prefix: string; rules: RuleData[] }> = [];
    for (const cat of categories) {
      const catRules = filtered.filter((r) => r.category === cat.name);
      if (catRules.length > 0) {
        groups.push({ category: cat.name, prefix: cat.prefix, rules: catRules });
      }
    }
    return groups;
  }, [filtered]);

  return (
    <div>
      {/* Search and filters */}
      <div className="mb-6 flex flex-col gap-4 sm:flex-row sm:items-center">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-fg-secondary" />
          <input
            type="text"
            placeholder="Search rules..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full rounded-lg border border-fg-dim/30 bg-bg-card py-2 pl-10 pr-4 text-sm text-fg-primary placeholder:text-fg-secondary focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
          />
        </div>
        <div className="flex flex-wrap gap-1.5">
          {["All", ...categories.map((c) => c.name)].map((cat) => {
            const isActive = selectedCategory === cat;
            return (
              <button
                key={cat}
                onClick={() => setSelectedCategory(cat)}
                className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                  isActive
                    ? "bg-accent/20 text-accent border border-accent/40"
                    : "border border-fg-dim/20 text-fg-secondary hover:text-fg-primary hover:border-fg-dim/40"
                }`}
              >
                {cat}
              </button>
            );
          })}
        </div>
      </div>

      {/* Results count */}
      <p className="mb-4 text-sm text-fg-secondary">
        {filtered.length} rule{filtered.length !== 1 ? "s" : ""}
      </p>

      {/* Table grouped by category */}
      {grouped.map((group) => (
        <div key={group.category} className="mb-8">
          <h2 className="mb-3 flex items-center gap-2 text-base font-semibold text-fg-primary">
            {group.category}
            <span className="text-xs font-normal text-fg-secondary">
              ({group.prefix}) &middot; {group.rules.length} rule{group.rules.length !== 1 ? "s" : ""}
            </span>
          </h2>
          <div className="overflow-x-auto rounded-lg border border-fg-dim/20">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-fg-dim/20 bg-bg-card/50">
                  <th className="px-3 py-2 text-left font-medium text-fg-secondary">Code</th>
                  <th className="px-3 py-2 text-left font-medium text-fg-secondary">Name</th>
                  <th className="hidden px-3 py-2 text-left font-medium text-fg-secondary md:table-cell">Message</th>
                  <th className="px-3 py-2 text-center font-medium text-fg-secondary">Status</th>
                </tr>
              </thead>
              <tbody>
                {group.rules.map((rule) => (
                  <tr
                    key={rule.code}
                    className="border-b border-fg-dim/10 last:border-b-0 hover:bg-bg-card/30 transition-colors"
                  >
                    <td className="px-3 py-2">
                      <Link
                        href={`/docs/rules/${rule.code}`}
                        className="font-mono text-accent hover:underline"
                      >
                        {rule.code}
                      </Link>
                    </td>
                    <td className="px-3 py-2">
                      <Link
                        href={`/docs/rules/${rule.code}`}
                        className="text-fg-primary hover:text-accent hover:underline"
                      >
                        {rule.name}
                      </Link>
                    </td>
                    <td className="hidden px-3 py-2 text-fg-secondary md:table-cell">
                      <span className="line-clamp-1">{rule.description}</span>
                    </td>
                    <td className="px-3 py-2 text-center">
                      {rule.implemented ? (
                        <span className="inline-block h-2 w-2 rounded-full bg-green-400" title="Implemented" />
                      ) : (
                        <span className="inline-block h-2 w-2 rounded-full bg-fg-dim/40" title="Planned" />
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      ))}

      {filtered.length === 0 && (
        <p className="py-12 text-center text-fg-secondary">
          No rules match your search.
        </p>
      )}
    </div>
  );
}
