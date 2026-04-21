import type { RuleStatus } from "@/app/lib/rules-data";

const categoryColors: Record<string, string> = {
  Correctness: "bg-red-500/15 text-red-400 border-red-500/25",
  Style: "bg-blue-500/15 text-blue-400 border-blue-500/25",
  Portability: "bg-purple-500/15 text-purple-400 border-purple-500/25",
  Performance: "bg-green-500/15 text-green-400 border-green-500/25",
  Security: "bg-amber-500/15 text-amber-400 border-amber-500/25",
};

export function CategoryBadge({ category }: { category: string }) {
  const colors = categoryColors[category] ?? "bg-fg-dim/10 text-fg-secondary border-fg-dim/20";
  return (
    <span className={`inline-flex items-center rounded-md border px-1.5 py-0.5 text-xs font-medium ${colors}`}>
      {category}
    </span>
  );
}

export function SeverityBadge({ severity }: { severity: string }) {
  const colors =
    severity === "Error"
      ? "bg-red-500/15 text-red-400 border-red-500/25"
      : "bg-yellow-500/15 text-yellow-400 border-yellow-500/25";
  return (
    <span className={`inline-flex items-center rounded-md border px-1.5 py-0.5 text-xs font-medium ${colors}`}>
      {severity}
    </span>
  );
}

const statusStyles: Record<RuleStatus, string> = {
  implemented: "border-green-500/25 bg-green-500/15 text-green-400",
  implemented_with_known_shellcheck_divergences:
    "border-yellow-500/25 bg-yellow-500/15 text-yellow-300",
  planned: "border-fg-dim/20 bg-fg-dim/10 text-fg-secondary",
};

const statusLabels: Record<RuleStatus, string> = {
  implemented: "Implemented",
  implemented_with_known_shellcheck_divergences:
    "Implemented with known ShellCheck divergences",
  planned: "Planned",
};

export function RuleStatusBadge({ status }: { status: RuleStatus }) {
  return (
    <span
      className={`inline-flex items-center rounded-md border px-1.5 py-0.5 text-xs font-medium ${statusStyles[status]}`}
    >
      {statusLabels[status]}
    </span>
  );
}

export function RuleStatusDot({ status }: { status: RuleStatus }) {
  const colors: Record<RuleStatus, string> = {
    implemented: "bg-green-400",
    implemented_with_known_shellcheck_divergences: "bg-yellow-300",
    planned: "bg-fg-dim/40",
  };

  return (
    <span
      className={`inline-block h-2 w-2 rounded-full ${colors[status]}`}
      title={statusLabels[status]}
      aria-label={statusLabels[status]}
    />
  );
}
