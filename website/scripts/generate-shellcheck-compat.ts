import {
  shellcheckCompatDataOutputPath,
  writeShellcheckCompatRepoData,
} from "./shellcheck-compat-data";

const repoData = writeShellcheckCompatRepoData();
const knownIssueCount = repoData.filter(
  (repo) => repo.status === "known_issues",
).length;

console.log(
  `Generated ${repoData.length} repository conformance rows (${knownIssueCount} with known issues) -> ${shellcheckCompatDataOutputPath}`,
);
