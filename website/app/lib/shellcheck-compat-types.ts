export type ShellcheckCompatRepoStatus = "passes" | "known_issues";

export interface ShellcheckCompatRepoData {
  repo: string;
  repoKey: string;
  url: string;
  status: ShellcheckCompatRepoStatus;
  issueCount: number;
}
