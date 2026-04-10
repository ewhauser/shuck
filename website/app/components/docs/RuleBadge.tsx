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

export function ImplementedBadge({ implemented }: { implemented: boolean }) {
  if (implemented) {
    return (
      <span className="inline-flex items-center rounded-md border border-green-500/25 bg-green-500/15 px-1.5 py-0.5 text-xs font-medium text-green-400">
        Implemented
      </span>
    );
  }
  return (
    <span className="inline-flex items-center rounded-md border border-fg-dim/20 bg-fg-dim/10 px-1.5 py-0.5 text-xs font-medium text-fg-secondary">
      Planned
    </span>
  );
}
