import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseYaml } from "yaml";
import type {
  ShellcheckCompatRepoData,
  ShellcheckCompatRepoStatus,
} from "../app/lib/shellcheck-compat-types";

const __dirname = dirname(fileURLToPath(import.meta.url));

interface ReviewedDivergenceRecord {
  path_contains?: string;
  path_suffix?: string;
}

export interface CorpusMetadataDocument {
  review_all_divergences?: boolean;
  reviewed_divergences?: ReviewedDivergenceRecord[];
}

const repoRootDir = resolve(__dirname, "../..");
const corpusDownloadPath = join(repoRootDir, "scripts/corpus-download.sh");
const corpusMetadataDir = join(
  repoRootDir,
  "crates/shuck-cli/tests/testdata/corpus-metadata",
);
const largeCorpusPath = join(
  repoRootDir,
  "crates/shuck-cli/tests/large_corpus.rs",
);

export const shellcheckCompatDataOutputPath = join(
  repoRootDir,
  "website/app/lib/shellcheck-compat-data.generated.json",
);

export function parseCuratedRepoList(source: string): string[] {
  const reposMatch = source.match(/REPOS="\n?([\s\S]*?)"/u);
  if (!reposMatch) {
    return [];
  }

  const repos: string[] = [];
  const seen = new Set<string>();

  for (const line of reposMatch[1].split(/\r?\n/u)) {
    const repo = line.trim();
    if (!repo || seen.has(repo)) {
      continue;
    }

    seen.add(repo);
    repos.push(repo);
  }

  return repos;
}

export function repoKeyFromRepo(repo: string): string {
  return repo.replace("/", "__");
}

export function normalizeMetadataPath(pathValue: string): string {
  return pathValue.replace(/^(?:corpus\/)?scripts\//u, "");
}

export function extractRepoKeyFromMetadataPath(
  pathValue: string,
): string | null {
  const normalized = normalizeMetadataPath(pathValue);
  const [owner, name] = normalized.split("__");

  if (!owner || !name) {
    return null;
  }

  return `${owner}__${name}`;
}

export function collectKnownIssueRepoKeys(
  metadataDocuments: CorpusMetadataDocument[],
): Set<string> {
  return new Set(collectKnownIssueRepoCounts(metadataDocuments).keys());
}

export function collectKnownIssueRepoCounts(
  metadataDocuments: CorpusMetadataDocument[],
): Map<string, number> {
  const repoCounts = new Map<string, number>();

  for (const document of metadataDocuments) {
    for (const divergence of document.reviewed_divergences ?? []) {
      for (const pathValue of [
        divergence.path_contains,
        divergence.path_suffix,
      ]) {
        if (typeof pathValue !== "string") {
          continue;
        }

        const repoKey = extractRepoKeyFromMetadataPath(pathValue);
        if (repoKey) {
          repoCounts.set(repoKey, (repoCounts.get(repoKey) ?? 0) + 1);
          break;
        }
      }
    }
  }

  return repoCounts;
}

export function parseUnsupportedRepoKeys(source: string): string[] {
  const match = source.match(
    /const LARGE_CORPUS_SHELLCHECK_UNSUPPORTED_REPO_PREFIXES: &\[&str\] = &\[([\s\S]*?)\];/u,
  );
  const body = match?.[1] ?? "";
  const repoKeys: string[] = [];
  const seen = new Set<string>();

  for (const literal of body.matchAll(/"([^"]+)"/gu)) {
    const repoKey = literal[1].replace(/__+$/u, "");
    if (!repoKey || seen.has(repoKey)) {
      continue;
    }

    seen.add(repoKey);
    repoKeys.push(repoKey);
  }

  return repoKeys;
}

export function buildShellcheckCompatRepoData(input: {
  curatedRepos: string[];
  knownIssueRepoCounts: Map<string, number>;
  unsupportedRepoKeys: Set<string>;
}): ShellcheckCompatRepoData[] {
  return input.curatedRepos
    .map((repo) => {
      const repoKey = repoKeyFromRepo(repo);
      const issueCount =
        (input.knownIssueRepoCounts.get(repoKey) ?? 0) +
        (input.unsupportedRepoKeys.has(repoKey) ? 1 : 0);
      const status: ShellcheckCompatRepoStatus = issueCount > 0
        ? "known_issues"
        : "passes";

      return {
        repo,
        repoKey,
        url: `https://github.com/${repo}`,
        status,
        issueCount,
      };
    })
    .sort((left, right) => left.repo.localeCompare(right.repo));
}

function loadCorpusMetadataDocuments(
  metadataDirPath: string = corpusMetadataDir,
): CorpusMetadataDocument[] {
  return readdirSync(metadataDirPath)
    .filter((file) => file.endsWith(".yaml"))
    .map((file) => {
      const content = readFileSync(join(metadataDirPath, file), "utf-8");
      return (parseYaml(content) as CorpusMetadataDocument | null) ?? {};
    });
}

export function generateShellcheckCompatRepoData(
  rootDir: string = repoRootDir,
): ShellcheckCompatRepoData[] {
  const curatedRepos = parseCuratedRepoList(
    readFileSync(join(rootDir, "scripts/corpus-download.sh"), "utf-8"),
  );
  const knownIssueRepoCounts = collectKnownIssueRepoCounts(
    loadCorpusMetadataDocuments(
      join(rootDir, "crates/shuck-cli/tests/testdata/corpus-metadata"),
    ),
  );
  const unsupportedRepoKeys = new Set(
    parseUnsupportedRepoKeys(
      readFileSync(join(rootDir, "crates/shuck-cli/tests/large_corpus.rs"), "utf-8"),
    ),
  );

  return buildShellcheckCompatRepoData({
    curatedRepos,
    knownIssueRepoCounts,
    unsupportedRepoKeys,
  });
}

export function writeShellcheckCompatRepoData(
  outputPath: string = shellcheckCompatDataOutputPath,
): ShellcheckCompatRepoData[] {
  const repoData = generateShellcheckCompatRepoData();
  writeFileSync(outputPath, JSON.stringify(repoData, null, 2) + "\n");
  return repoData;
}

export const shellcheckCompatSourcePaths = {
  corpusDownloadPath,
  corpusMetadataDir,
  largeCorpusPath,
};
