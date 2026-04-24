import { shellcheckCompatRepos } from "@/app/lib/shellcheck-compat-data";
import type {
  ShellcheckCompatRepoData,
  ShellcheckCompatRepoStatus,
} from "@/app/lib/shellcheck-compat-types";

const statusLabels: Record<ShellcheckCompatRepoStatus, string> = {
  passes: "Passes checked-in corpus conformance",
  known_issues: "Known reviewed divergences or ShellCheck unsupported",
};

const statusColors: Record<ShellcheckCompatRepoStatus, string> = {
  passes: "bg-green-400",
  known_issues: "bg-yellow-300",
};

function issueCountLabel(issueCount: number) {
  return `${issueCount} known issue${issueCount === 1 ? "" : "s"}`;
}

function statusLabel(repo: ShellcheckCompatRepoData) {
  if (repo.status === "known_issues") {
    return issueCountLabel(repo.issueCount);
  }

  return statusLabels[repo.status];
}

function StatusDot({ repo }: { repo: ShellcheckCompatRepoData }) {
  const label = statusLabel(repo);

  return (
    <span className="group relative inline-flex items-center justify-center">
      <span
        tabIndex={0}
        className={`inline-block h-2.5 w-2.5 rounded-full outline-none ${statusColors[repo.status]} focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-bg-card`}
        aria-label={label}
      />
      <span
        role="tooltip"
        className="pointer-events-none absolute left-1/2 top-full z-10 mt-2 -translate-x-1/2 whitespace-nowrap rounded-md border border-fg-dim/30 bg-bg-card px-2 py-1 text-xs text-fg-primary opacity-0 shadow-lg transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
      >
        {label}
      </span>
    </span>
  );
}

export default function ShellcheckCompatRepoTable() {
  return (
    <div className="my-6">
      <div className="mb-3 flex flex-wrap gap-4 text-sm text-fg-secondary">
        <span className="inline-flex items-center gap-2">
          <span className="inline-block h-2.5 w-2.5 rounded-full bg-green-400" aria-hidden="true" />
          Passes checked-in corpus conformance
        </span>
        <span className="inline-flex items-center gap-2">
          <span className="inline-block h-2.5 w-2.5 rounded-full bg-yellow-300" aria-hidden="true" />
          Known reviewed divergences or ShellCheck unsupported
        </span>
      </div>

      <div className="overflow-x-auto rounded-lg border border-fg-dim/20">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-fg-dim/20 bg-bg-card/50">
              <th className="px-3 py-2 text-left font-medium text-fg-secondary">
                Repository
              </th>
              <th className="w-20 px-3 py-2 text-center font-medium text-fg-secondary">
                Status
              </th>
            </tr>
          </thead>
          <tbody>
            {shellcheckCompatRepos.map((repo) => (
              <tr
                key={repo.repo}
                className="border-b border-fg-dim/10 last:border-b-0 hover:bg-bg-card/30 transition-colors"
              >
                <td className="px-3 py-2">
                  <a
                    href={repo.url}
                    className="text-fg-primary hover:text-accent hover:underline"
                  >
                    {repo.repo}
                  </a>
                </td>
                <td className="px-3 py-2 text-center">
                  <span className="inline-flex items-center justify-center">
                    <StatusDot repo={repo} />
                    <span className="sr-only">{statusLabel(repo)}</span>
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
