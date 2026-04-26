import localSnapshotJson from "@/generated/benchmarks/local-m5-max.json";
import ciSnapshotJson from "@/generated/benchmarks/ci-latest.json";
import type { BenchmarkDataset, BenchmarkMeasurement } from "@/app/lib/benchmarks";
import {
  describeRelativePerformance,
  formatBytes,
  formatDate,
  formatDuration,
  formatInteger,
  formatMemory,
  formatRatio,
  getAggregateCase,
  getComparisonMeasurement,
  getMeasurement,
  pickLatestDatasetWithCorpus,
} from "@/app/lib/benchmarks";

const localSnapshot = localSnapshotJson as BenchmarkDataset;
const ciSnapshot = ciSnapshotJson as BenchmarkDataset;
const snapshots = [localSnapshot, ciSnapshot];
const corpus = (pickLatestDatasetWithCorpus(snapshots) ?? localSnapshot).corpus;

function measurementText(measurement: BenchmarkMeasurement | undefined) {
  if (!measurement) {
    return "n/a";
  }

  return `${formatDuration(measurement.meanSeconds)} (+/- ${formatDuration(
    measurement.stddevSeconds,
  )})`;
}

function datasetLabel(dataset: BenchmarkDataset) {
  return dataset.environment.kind === "ci" ? "CI snapshot" : "Checked-in local snapshot";
}

function OverviewTable() {
  return (
    <table>
      <thead>
        <tr>
          <th>Snapshot</th>
          <th>Environment</th>
          <th>Commit</th>
          <th>shuck (all)</th>
          <th>Comparison (all)</th>
          <th>Speedup</th>
        </tr>
      </thead>
      <tbody>
        {snapshots.map((dataset) => {
          const aggregate = getAggregateCase(dataset);
          const shuck = aggregate ? getMeasurement(aggregate, "shuck") : undefined;
          const comparison = aggregate ? getComparisonMeasurement(aggregate) : undefined;

          return (
            <tr key={dataset.id}>
              <td>
                <strong>{dataset.name}</strong>
                <br />
                <span>{datasetLabel(dataset)}</span>
              </td>
              <td>
                {dataset.environment.label}
                <br />
                <span>
                  {dataset.environment.os} ({dataset.environment.arch})
                </span>
              </td>
              <td>
                {dataset.links.commitUrl && dataset.commit.shortSha ? (
                  <a href={dataset.links.commitUrl}>{dataset.commit.shortSha}</a>
                ) : (
                  dataset.commit.shortSha ?? "n/a"
                )}
                <br />
                <span>{formatDate(dataset.generatedAt)}</span>
              </td>
              <td>{dataset.available ? measurementText(shuck) : "not generated in this build"}</td>
              <td>
                {dataset.available ? measurementText(comparison) : "not generated in this build"}
              </td>
              <td>
                {dataset.available ? formatRatio(dataset.summary?.speedupRatio) : "n/a"}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function SnapshotDetails({ dataset }: { dataset: BenchmarkDataset }) {
  const aggregate = getAggregateCase(dataset);
  const shuck = aggregate ? getMeasurement(aggregate, "shuck") : undefined;
  const comparison = aggregate ? getComparisonMeasurement(aggregate) : undefined;

  return (
    <>
      <h2>{dataset.name}</h2>
      <p>{dataset.description}</p>

      <table>
        <tbody>
          <tr>
            <th>Environment</th>
            <td>{dataset.environment.label}</td>
          </tr>
          <tr>
            <th>OS / arch</th>
            <td>
              {dataset.environment.os} ({dataset.environment.arch})
            </td>
          </tr>
          <tr>
            <th>CPU</th>
            <td>{dataset.environment.cpu ?? "Unknown"}</td>
          </tr>
          <tr>
            <th>Generated</th>
            <td>{formatDate(dataset.generatedAt)}</td>
          </tr>
          <tr>
            <th>Commit</th>
            <td>
              {dataset.links.commitUrl && dataset.commit.shortSha ? (
                <a href={dataset.links.commitUrl}>{dataset.commit.shortSha}</a>
              ) : (
                dataset.commit.shortSha ?? "n/a"
              )}
            </td>
          </tr>
          <tr>
            <th>Runner</th>
            <td>
              hyperfine {dataset.methodology.warmupRuns} warmups /{" "}
              {dataset.methodology.measuredRuns} measured runs
            </td>
          </tr>
          <tr>
            <th>shuck</th>
            <td>{dataset.toolVersions.shuck ?? "Unknown"}</td>
          </tr>
          <tr>
            <th>{comparison ? comparison.tool : "Comparison tool"}</th>
            <td>
              {comparison?.tool === "shellcheck"
                ? dataset.toolVersions.shellcheck ?? "Unknown"
                : comparison?.tool ?? "Not included"}
            </td>
          </tr>
          <tr>
            <th>Aggregate result</th>
            <td>
              {dataset.available && shuck
                ? `${measurementText(shuck)}`
                : "This build does not include a generated aggregate result."}
            </td>
          </tr>
          <tr>
            <th>Aggregate speedup</th>
            <td>
              {dataset.available && comparison
                ? describeRelativePerformance(
                    dataset.summary?.speedupRatio,
                    comparison.tool,
                  )
                : "n/a"}
            </td>
          </tr>
          {dataset.links.runUrl ? (
            <tr>
              <th>Workflow run</th>
              <td>
                <a href={dataset.links.runUrl}>GitHub Actions run</a>
              </td>
            </tr>
          ) : null}
        </tbody>
      </table>

      {!dataset.available ? (
        <blockquote>
          The checked-in placeholder was used for this build. The GitHub Pages
          deployment for the latest published release regenerates the CI snapshot
          before exporting the site.
        </blockquote>
      ) : null}
    </>
  );
}

function CaseTable({ dataset }: { dataset: BenchmarkDataset }) {
  if (!dataset.available) {
    return null;
  }

  return (
    <>
      <h3>{dataset.environment.kind === "ci" ? "Linux CI results" : "Apple M5 results"}</h3>
      <table>
        <thead>
          <tr>
            <th>Case</th>
            <th>Size</th>
            <th>shuck</th>
            <th>Comparison</th>
            <th>Speedup</th>
            <th>shuck RSS</th>
            <th>Comparison RSS</th>
          </tr>
        </thead>
        <tbody>
          {dataset.cases.map((benchmarkCase) => {
            const shuck = getMeasurement(benchmarkCase, "shuck");
            const comparison = getComparisonMeasurement(benchmarkCase);
            return (
              <tr key={benchmarkCase.slug}>
                <td>
                  <strong>
                    {benchmarkCase.slug === "all"
                      ? "all"
                      : benchmarkCase.fixture?.fileName ?? benchmarkCase.slug}
                  </strong>
                  <br />
                  <span>
                    {benchmarkCase.slug === "all"
                      ? `${formatInteger(benchmarkCase.fixtureCount)} files in one invocation`
                      : benchmarkCase.fixture?.upstreamRepo}
                  </span>
                </td>
                <td>
                  {formatBytes(benchmarkCase.bytes)}
                  <br />
                  <span>
                    {benchmarkCase.lines != null
                      ? `${formatInteger(benchmarkCase.lines)} lines`
                      : "n/a"}
                  </span>
                </td>
                <td>{measurementText(shuck)}</td>
                <td>{measurementText(comparison)}</td>
                <td>{formatRatio(comparison?.relativeToShuck)}</td>
                <td>{formatMemory(shuck?.meanMemoryBytes)}</td>
                <td>{formatMemory(comparison?.meanMemoryBytes)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </>
  );
}

export default function BenchmarksDoc() {
  return (
    <>
      <h1>Benchmarks</h1>

      <p>
        This page publishes the macro CLI benchmark results produced by{" "}
        <code>make bench-macro</code>. It is intended as a technical reference for
        engineers evaluating runtime characteristics, not as a cross-machine league
        table.
      </p>

      <p>
        Two datasets are shown:
      </p>

      <ul>
        <li>
          A checked-in Apple M5 Max snapshot generated from local benchmark exports.
        </li>
        <li>
          A Linux CI snapshot regenerated during the GitHub Pages deploy for the
          latest published release.
        </li>
      </ul>

      <p>
        Compare tools within the same snapshot. Absolute numbers across different
        machines are useful for rough orientation only.
      </p>

      <h2>Snapshot Overview</h2>
      <OverviewTable />

      <h2>Methodology</h2>

      <ul>
        <li>
          The runner is <code>hyperfine</code> with{" "}
          <code>{localSnapshot.methodology.warmupRuns}</code> warmups and{" "}
          <code>{localSnapshot.methodology.measuredRuns}</code> measured runs per case.
        </li>
        <li>
          <code>shuck</code> is measured with <code>check --no-cache</code> so the
          results reflect parsing and linting work rather than cache reuse.
        </li>
        <li>
          The comparison command is{" "}
          <code>shellcheck --enable=all --severity=style &lt;fixture&gt;</code> on
          the same input.
        </li>
        <li>
          <code>--ignore-failure</code> is intentional. These fixtures contain lint
          findings, so non-zero exit codes are expected and the benchmark is measuring
          runtime rather than success state.
        </li>
        <li>
          Each fixture is benchmarked independently, and the <code>all</code> case
          benchmarks one invocation over the entire vendored corpus.
        </li>
      </ul>

      <h2>Reproducing Results</h2>

      <h3>Refresh the checked-in local snapshot</h3>
      <pre>
        <code>make bench-macro-site-local</code>
      </pre>

      <h3>Generate a CI-style dataset manually</h3>
      <pre>
        <code>{`./scripts/benchmarks/setup.sh hyperfine shellcheck
./scripts/benchmarks/run.sh
python3 ./scripts/benchmarks/export_website_data.py \\
  --repo-root . \\
  --bench-dir .cache \\
  --output website/generated/benchmarks/ci-latest.json \\
  --dataset-id ci-latest \\
  --dataset-name "GitHub Actions latest release snapshot" \\
  --dataset-description "Generated during the website deploy workflow for the latest published release." \\
  --environment-kind ci \\
  --environment-label "GitHub Actions ubuntu-latest"`}</code>
      </pre>

      {snapshots.map((dataset) => (
        <div key={dataset.id}>
          <SnapshotDetails dataset={dataset} />
          <CaseTable dataset={dataset} />
        </div>
      ))}

      <h2>Benchmark Corpus</h2>

      <p>
        The benchmark corpus is vendored into the repository so every run uses the
        same inputs. The current corpus contains {formatInteger(corpus.fixtureCount)}{" "}
        files, {formatInteger(corpus.totalLines)} lines, and {formatBytes(corpus.totalBytes)}{" "}
        of shell source.
      </p>

      <table>
        <thead>
          <tr>
            <th>Fixture</th>
            <th>Source</th>
            <th>Size</th>
            <th>License</th>
          </tr>
        </thead>
        <tbody>
          {corpus.fixtures.map((fixture) => (
            <tr key={fixture.slug}>
              <td>
                <strong>{fixture.fileName}</strong>
                <br />
                <span>{fixture.commitShort}</span>
              </td>
              <td>
                <a href={fixture.sourceUrl}>{fixture.upstreamRepo}</a>
                <br />
                <span>{fixture.upstreamPath}</span>
              </td>
              <td>
                {formatBytes(fixture.bytes)}
                <br />
                <span>{formatInteger(fixture.lines)} lines</span>
              </td>
              <td>{fixture.license}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
