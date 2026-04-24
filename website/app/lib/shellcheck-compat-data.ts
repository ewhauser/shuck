import shellcheckCompatRepoJson from "./shellcheck-compat-data.generated.json";
import type { ShellcheckCompatRepoData } from "./shellcheck-compat-types";

export const shellcheckCompatRepos =
  shellcheckCompatRepoJson as ShellcheckCompatRepoData[];
