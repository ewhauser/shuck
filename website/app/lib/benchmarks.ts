export type BenchmarkFixture = {
  slug: string;
  name: string;
  fileName: string;
  path: string;
  bytes: number;
  lines: number;
  upstreamRepo: string;
  upstreamPath: string;
  sourceUrl: string;
  license: string;
  commit: string;
  commitShort: string;
};

export type BenchmarkMeasurement = {
  tool: string;
  command: string;
  meanSeconds: number;
  stddevSeconds: number;
  medianSeconds: number;
  minSeconds: number;
  maxSeconds: number;
  userSeconds: number;
  systemSeconds: number;
  meanMemoryBytes: number | null;
  maxMemoryBytes: number | null;
  runCount: number;
  exitCodes: number[];
  hasFailures: boolean;
  relativeToShuck: number | null;
};

export type BenchmarkCase = {
  slug: string;
  name: string;
  kind: "aggregate" | "fixture";
  bytes: number | null;
  lines: number | null;
  fixtureCount: number;
  measurements: BenchmarkMeasurement[];
  fixture?: BenchmarkFixture;
};

export type BenchmarkDataset = {
  schemaVersion: number;
  available: boolean;
  id: string;
  name: string;
  description: string;
  generatedAt: string;
  commit: {
    sha: string | null;
    shortSha: string | null;
  };
  links: {
    repositoryUrl: string | null;
    commitUrl: string | null;
    runUrl: string | null;
  };
  environment: {
    kind: "local" | "ci";
    label: string;
    os: string;
    arch: string;
    cpu: string | null;
    python: string;
  };
  toolVersions: {
    shuck: string | null;
    hyperfine: string | null;
    shellcheck: string | null;
  };
  methodology: {
    benchmarkCommand: string;
    benchmarkMode: "compare" | "shuck-only";
    warmupRuns: number;
    measuredRuns: number;
    ignoreFailure: boolean;
    shuckCommand: string;
    comparisonCommand: string | null;
    notes: string | null;
  };
  corpus: {
    fixtureCount: number;
    totalBytes: number;
    totalLines: number;
    fixtures: BenchmarkFixture[];
  };
  summary: {
    aggregateCase: string;
    primaryTool: string;
    comparisonTool: string | null;
    shuckMeanSeconds: number;
    comparisonMeanSeconds: number | null;
    speedupRatio: number | null;
    timeSavedPct: number | null;
  } | null;
  cases: BenchmarkCase[];
};

export function pickLatestDatasetWithCorpus(
  datasets: BenchmarkDataset[],
): BenchmarkDataset | undefined {
  return datasets
    .filter((dataset) => dataset.corpus.fixtures.length > 0)
    .sort(
      (left, right) =>
        Date.parse(right.generatedAt) - Date.parse(left.generatedAt),
    )[0];
}

export function getAggregateCase(dataset: BenchmarkDataset): BenchmarkCase | undefined {
  return dataset.cases.find((candidate) => candidate.slug === "all");
}

export function getMeasurement(
  benchmarkCase: BenchmarkCase,
  tool: string,
): BenchmarkMeasurement | undefined {
  return benchmarkCase.measurements.find((candidate) => candidate.tool === tool);
}

export function getComparisonMeasurement(
  benchmarkCase: BenchmarkCase,
): BenchmarkMeasurement | undefined {
  return benchmarkCase.measurements.find((candidate) => candidate.tool !== "shuck");
}

export function formatDuration(seconds: number): string {
  if (seconds >= 1) {
    return `${seconds.toFixed(2)} s`;
  }
  if (seconds >= 0.001) {
    return `${(seconds * 1000).toFixed(1)} ms`;
  }
  return `${(seconds * 1000000).toFixed(1)} us`;
}

export function formatRatio(ratio: number | null | undefined): string {
  if (!ratio || !Number.isFinite(ratio)) {
    return "n/a";
  }
  if (ratio >= 100) {
    return `${ratio.toFixed(1)}x`;
  }
  return `${ratio.toFixed(2)}x`;
}

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) {
    return "n/a";
  }
  if (bytes >= 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  }
  if (bytes >= 1024) {
    return `${(bytes / 1024).toFixed(1)} KiB`;
  }
  return `${bytes} B`;
}

export function formatMemory(bytes: number | null | undefined): string {
  if (bytes == null) {
    return "n/a";
  }
  if (bytes >= 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
  }
  return formatBytes(bytes);
}

export function formatDate(isoString: string): string {
  return new Intl.DateTimeFormat("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: "UTC",
  }).format(new Date(isoString));
}

export function formatInteger(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}
