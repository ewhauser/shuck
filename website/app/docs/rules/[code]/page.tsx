import { notFound } from "next/navigation";
import type { Metadata } from "next";
import Link from "next/link";
import { ChevronRight } from "lucide-react";
import { codeToHtml } from "shiki";
import { allRules, getRuleByCode } from "@/app/lib/rules-data";
import { CategoryBadge, SeverityBadge, ImplementedBadge } from "@/app/components/docs/RuleBadge";

interface Props {
  params: Promise<{ code: string }>;
}

export async function generateStaticParams() {
  return allRules.map((r) => ({ code: r.code }));
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const { code } = await params;
  const rule = getRuleByCode(code);
  if (!rule) return {};
  return {
    title: `${rule.code}: ${rule.name}`,
    description: rule.description,
  };
}

export default async function RuleDetailPage({ params }: Props) {
  const { code } = await params;
  const rule = getRuleByCode(code);

  if (!rule) {
    notFound();
  }

  // Highlight examples
  const highlightedExamples = await Promise.all(
    rule.examples.map(async (ex) => ({
      kind: ex.kind,
      html: await codeToHtml(ex.code, {
        lang: "bash",
        theme: "github-dark-default",
      }),
    })),
  );

  return (
    <div className="max-w-3xl">
      {/* Breadcrumb */}
      <nav className="mb-6 flex items-center gap-1 text-sm text-fg-secondary">
        <Link href="/docs/getting-started" className="hover:text-fg-primary transition-colors">
          Docs
        </Link>
        <ChevronRight className="h-3.5 w-3.5" />
        <Link href="/docs/rules" className="hover:text-fg-primary transition-colors">
          Rules
        </Link>
        <ChevronRight className="h-3.5 w-3.5" />
        <span className="text-fg-primary">{rule.code}</span>
      </nav>

      {/* Title */}
      <h1 className="mb-3 text-2xl font-bold text-fg-primary">
        {rule.code}: {rule.name}
      </h1>

      {/* Badges */}
      <div className="mb-6 flex flex-wrap gap-2">
        <CategoryBadge category={rule.category} />
        {rule.severity && <SeverityBadge severity={rule.severity} />}
        <ImplementedBadge implemented={rule.implemented} />
      </div>

      {/* Description */}
      <section className="mb-6">
        <h2 className="mb-2 text-lg font-semibold text-fg-primary">What it does</h2>
        <p className="text-fg-secondary leading-relaxed">{rule.description}</p>
      </section>

      {/* Rationale */}
      <section className="mb-6">
        <h2 className="mb-2 text-lg font-semibold text-fg-primary">Why is this bad?</h2>
        <p className="text-fg-secondary leading-relaxed">{rule.rationale}</p>
      </section>

      {/* Examples */}
      {highlightedExamples.length > 0 && (
        <section className="mb-6">
          <h2 className="mb-2 text-lg font-semibold text-fg-primary">Example</h2>
          {highlightedExamples.map((ex, i) => (
            <div key={i} className="mb-3">
              {ex.kind === "invalid" && (
                <p className="mb-1 text-xs font-medium text-red-400">Bad</p>
              )}
              {ex.kind === "valid" && (
                <p className="mb-1 text-xs font-medium text-green-400">Good</p>
              )}
              <div
                className="overflow-x-auto rounded-lg border border-fg-dim/20 text-sm [&_pre]:p-4"
                dangerouslySetInnerHTML={{ __html: ex.html }}
              />
            </div>
          ))}
        </section>
      )}

      {/* Metadata */}
      <section className="rounded-lg border border-fg-dim/20 bg-bg-card/30 p-4">
        <h2 className="mb-3 text-sm font-semibold text-fg-primary">Details</h2>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
          {rule.shellcheckCode && (
            <>
              <dt className="text-fg-secondary">ShellCheck</dt>
              <dd className="font-mono text-fg-primary">{rule.shellcheckCode}</dd>
            </>
          )}
          <dt className="text-fg-secondary">Shells</dt>
          <dd className="flex flex-wrap gap-1">
            {rule.shells.map((s) => (
              <span
                key={s}
                className="rounded border border-fg-dim/20 bg-fg-dim/10 px-1.5 py-0.5 font-mono text-xs text-fg-primary"
              >
                {s}
              </span>
            ))}
          </dd>
          {rule.severity && (
            <>
              <dt className="text-fg-secondary">Severity</dt>
              <dd className="text-fg-primary">{rule.severity}</dd>
            </>
          )}
          <dt className="text-fg-secondary">Category</dt>
          <dd className="text-fg-primary">{rule.category}</dd>
        </dl>
      </section>
    </div>
  );
}
