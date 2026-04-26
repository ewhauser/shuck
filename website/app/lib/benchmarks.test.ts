import assert from "node:assert/strict";
import test from "node:test";
import {
  describeRelativePerformance,
  pickLatestDatasetWithCorpus,
  type BenchmarkDataset,
} from "./benchmarks";

const baseDataset: BenchmarkDataset = {
  schemaVersion: 1,
  available: true,
  id: "base",
  name: "Base dataset",
  description: "test dataset",
  generatedAt: "2026-04-20T17:00:00Z",
  commit: {
    sha: null,
    shortSha: null,
  },
  links: {
    repositoryUrl: null,
    commitUrl: null,
    runUrl: null,
  },
  environment: {
    kind: "local",
    label: "label",
    os: "os",
    arch: "arch",
    cpu: null,
    python: "3.13.0",
  },
  toolVersions: {
    shuck: null,
    hyperfine: null,
    shellcheck: null,
  },
  methodology: {
    benchmarkCommand: "make bench-macro",
    benchmarkMode: "compare",
    warmupRuns: 3,
    measuredRuns: 10,
    ignoreFailure: true,
    shuckCommand: "shuck check --no-cache --select ALL <fixture>",
    comparisonCommand: "shellcheck --enable=all --severity=style <fixture>",
    notes: null,
  },
  corpus: {
    fixtureCount: 0,
    totalBytes: 0,
    totalLines: 0,
    fixtures: [],
  },
  summary: null,
  cases: [],
};

test("pickLatestDatasetWithCorpus prefers the freshest dataset with fixtures", () => {
  const localDataset: BenchmarkDataset = {
    ...baseDataset,
    id: "local",
    generatedAt: "2026-04-20T17:00:00Z",
    corpus: {
      fixtureCount: 1,
      totalBytes: 10,
      totalLines: 1,
      fixtures: [
        {
          slug: "local",
          name: "local",
          fileName: "local.sh",
          path: "files/local.sh",
          bytes: 10,
          lines: 1,
          upstreamRepo: "example/local",
          upstreamPath: "local.sh",
          sourceUrl: "https://example.invalid/local.sh",
          license: "MIT",
          commit: "abc1234",
          commitShort: "abc1234",
        },
      ],
    },
  };
  const ciDataset: BenchmarkDataset = {
    ...baseDataset,
    id: "ci",
    generatedAt: "2026-04-20T18:00:00Z",
    environment: {
      ...baseDataset.environment,
      kind: "ci",
    },
    corpus: {
      fixtureCount: 1,
      totalBytes: 20,
      totalLines: 2,
      fixtures: [
        {
          slug: "ci",
          name: "ci",
          fileName: "ci.sh",
          path: "files/ci.sh",
          bytes: 20,
          lines: 2,
          upstreamRepo: "example/ci",
          upstreamPath: "ci.sh",
          sourceUrl: "https://example.invalid/ci.sh",
          license: "MIT",
          commit: "def5678",
          commitShort: "def5678",
        },
      ],
    },
  };

  const chosen = pickLatestDatasetWithCorpus([localDataset, ciDataset]);

  assert.equal(chosen?.id, "ci");
  assert.equal(chosen?.corpus.fixtures[0]?.slug, "ci");
});

test("describeRelativePerformance reports faster ratios correctly", () => {
  assert.equal(
    describeRelativePerformance(1.5, "shellcheck"),
    "1.50x faster than shellcheck",
  );
});

test("describeRelativePerformance reports slower ratios correctly", () => {
  assert.equal(
    describeRelativePerformance(0.5, "shellcheck"),
    "2.00x slower than shellcheck",
  );
});

test("describeRelativePerformance reports ties neutrally", () => {
  assert.equal(
    describeRelativePerformance(1, "shellcheck"),
    "roughly tied with shellcheck",
  );
});
