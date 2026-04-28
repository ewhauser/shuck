# S001 Reviewed Divergence Bugs

This file classifies the current reviewed S001/SC2086 large-corpus divergences.
The source of truth was a fresh `make large-corpus-report SHUCK_LARGE_CORPUS_RULES=S001`
run with ShellCheck 0.11.0.

Current summary from `target/large-corpus-report/latest.log`:

- `implementation_diffs=0`
- `mapping_issues=0`
- `reviewed_divergences=13`
- Individual reviewed records classified below: 14 total (`shellcheck-only=8`, `shuck-only=6`).

The harness summary counts reviewed divergence groups. This document lists the individual
diagnostic records that need to be cleared.

When a record is fixed, remove the matching entry from
`crates/shuck-cli/tests/testdata/corpus-metadata/s001.yaml` in the same change.
Resolved records should not remain as reviewed divergences.

## [ ] ShellCheck-only: semantically safe in Shuck (8)

- `233boy__v2ray__src__core.sh:1254:39-43` `$net`
  The final token is assembled from fixed protocol and transport fragments before it is passed onward.
- `RetroPie__RetroPie-Setup__scriptmodules__supplementary__runcommand__runcommand.sh:235:32-38` `$group`
  `group` is a static loop variable ranging over `CEA` and `DMT`.
- `gentoo__gentoo__eclass__tests__toolchain.sh:155:7-13` `${ret}`
  `ret` is a numeric status slot that is set to `1` on failure before the helper call.
- `juewuy__ShellCrash__scripts__menus__9_upgrade.sh:651:70-80` `${project}`
  `project` is selected from a fixed repository enum before the GitHub API URL is built.
- `juewuy__ShellCrash__scripts__menus__9_upgrade.sh:867:52-62` `${db_type}`
  `db_type` comes from a fixed dashboard selector before the archive path is assembled.
- `rvm__rvm__binscripts__rvm-installer:89:24-48` `${rvm_tar_command:-gtar}`
  This use sits inside the `else` arm where `rvm_tar_command` only comes from literal tar command names.
- `rvm__rvm__scripts__functions__manage__macruby:145:14-21` `$result`
  ShellCheck only reports this when the dead branch tail after an earlier `return 1` remains in the file.
- `tteck__Proxmox__vm__nextcloud-vm.sh:210:96-99` `$HN`
  `HN` is the current hostname seed used as a whiptail default value before any user edits.

## [ ] Shuck-only: broader modeling gaps or command-aware ShellCheck suppressions (6)

- `bats-core__bats-core__libexec__bats-core__bats-format-pretty:78:13-32` `$line_backoff_count`
  `move_up` consumes its first parameter numerically, but Shuck does not yet propagate numeric function-argument contracts back to callers.
- `bittorf__kalua__openwrt-monitoring__meshrdf_generate_table.sh:3374:6-27` `${inet_offer_down:-0}`
  ShellCheck stays quiet on this numeric-default conditional proof; Shuck still flags the expansion.
- `ko1nksm__shellspec__lib__general.sh:442:15-39` `$shellspec_readfile_data`
  The data is intentionally expanded through `set --` after eval-built escaping, which ShellCheck suppresses but Shuck still flags.
- `masonr__yet-another-bench-script__yabs.sh:991:12-19` `$GB_URL`
  `GB_URL` is chosen from literal Geekbench download URLs before it is passed to the downloader command position.
- `pi-hole__pi-hole__automated install__basic-install.sh:1833:21-39` `${webInterfaceDir}`
  `webInterfaceDir` is a webroot-derived path passed into the repository helper, but Shuck still treats it as a generic argument.
- `swoodford__aws__vpc-sg-import-rules-cloudflare.sh:281:101-106` `$PORT`
  `PORT` is regex-validated as a numeric or range token before the AWS `--port` argument is built.
