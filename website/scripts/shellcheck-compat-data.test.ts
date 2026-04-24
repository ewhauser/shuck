import assert from "node:assert/strict";
import test from "node:test";
import {
  buildShellcheckCompatRepoData,
  collectKnownIssueRepoCounts,
  extractRepoKeyFromMetadataPath,
  generateShellcheckCompatRepoData,
  parseCuratedRepoList,
  parseUnsupportedRepoKeys,
} from "./shellcheck-compat-data";

test("parseCuratedRepoList dedupes repositories while preserving order", () => {
  const repos = parseCuratedRepoList(`REPOS="
foo/bar
baz/qux
foo/bar
"`);

  assert.deepEqual(repos, ["foo/bar", "baz/qux"]);
});

test("extractRepoKeyFromMetadataPath handles direct and prefixed corpus paths", () => {
  assert.equal(
    extractRepoKeyFromMetadataPath("termux__termux-packages__packages__dart__build.sh"),
    "termux__termux-packages",
  );
  assert.equal(
    extractRepoKeyFromMetadataPath("scripts/rvm__rvm__bin__rvmsudo"),
    "rvm__rvm",
  );
  assert.equal(
    extractRepoKeyFromMetadataPath("corpus/scripts/xwmx__nb__nb"),
    "xwmx__nb",
  );
});

test("collectKnownIssueRepoCounts maps path_contains and path_suffix entries back to repos", () => {
  const repoCounts = collectKnownIssueRepoCounts([
    {
      reviewed_divergences: [
        { path_contains: "scripts/rvm__rvm__" },
        {
          path_suffix:
            "corpus/scripts/termux__termux-packages__packages__dart__build.sh",
        },
      ],
    },
  ]);

  assert.equal(repoCounts.get("rvm__rvm"), 1);
  assert.equal(repoCounts.get("termux__termux-packages"), 1);
});

test("parseUnsupportedRepoKeys trims repo prefixes from the harness constant", () => {
  const repoKeys = parseUnsupportedRepoKeys(`
const LARGE_CORPUS_SHELLCHECK_UNSUPPORTED_REPO_PREFIXES: &[&str] = &[
    "ohmyzsh__ohmyzsh__",
    "foo__bar__",
];
`);

  assert.deepEqual(repoKeys, ["ohmyzsh__ohmyzsh", "foo__bar"]);
});

test("buildShellcheckCompatRepoData marks known-issue and unsupported repos yellow", () => {
  const repoData = buildShellcheckCompatRepoData({
    curatedRepos: ["ohmyzsh/ohmyzsh", "rbenv/rbenv", "rvm/rvm"],
    knownIssueRepoCounts: new Map([["rvm__rvm", 3]]),
    unsupportedRepoKeys: new Set(["ohmyzsh__ohmyzsh"]),
  });

  assert.equal(
    repoData.find((repo) => repo.repo === "ohmyzsh/ohmyzsh")?.status,
    "known_issues",
  );
  assert.equal(
    repoData.find((repo) => repo.repo === "ohmyzsh/ohmyzsh")?.issueCount,
    1,
  );
  assert.equal(
    repoData.find((repo) => repo.repo === "rvm/rvm")?.status,
    "known_issues",
  );
  assert.equal(
    repoData.find((repo) => repo.repo === "rvm/rvm")?.issueCount,
    3,
  );
  assert.equal(
    repoData.find((repo) => repo.repo === "rbenv/rbenv")?.status,
    "passes",
  );
  assert.equal(
    repoData.find((repo) => repo.repo === "rbenv/rbenv")?.issueCount,
    0,
  );
  assert.deepEqual(repoData.map((repo) => repo.repo), [
    "ohmyzsh/ohmyzsh",
    "rbenv/rbenv",
    "rvm/rvm",
  ]);
});

test("generateShellcheckCompatRepoData reflects the checked-in corpus sources", () => {
  const repoData = generateShellcheckCompatRepoData();

  assert.equal(
    repoData.find((repo) => repo.repo === "rvm/rvm")?.status,
    "known_issues",
  );
  assert.equal(
    repoData.find((repo) => repo.repo === "ohmyzsh/ohmyzsh")?.status,
    "known_issues",
  );
  assert.equal(
    repoData.find((repo) => repo.repo === "rbenv/rbenv")?.status,
    "passes",
  );
  assert.equal(
    repoData.filter((repo) => repo.repo === "xwmx/nb").length,
    1,
  );
  assert.ok(
    (repoData.find((repo) => repo.repo === "rvm/rvm")?.issueCount ?? 0) > 0,
  );
  assert.equal(
    repoData.find((repo) => repo.repo === "rbenv/rbenv")?.issueCount,
    0,
  );
});
