# S001 Reviewed Divergence Bugs

This file classifies the current reviewed S001/SC2086 large-corpus divergences.
The source of truth was a fresh `make large-corpus-report SHUCK_LARGE_CORPUS_RULES=S001`
run with ShellCheck 0.11.0.

Current summary from `target/large-corpus-report/latest.log`:

- `implementation_diffs=0`
- `mapping_issues=0`
- `reviewed_divergences=26`
- Individual reviewed records classified below: 34 total (`shellcheck-only=17`, `shuck-only=17`).

The harness summary counts reviewed divergence groups. This document lists the individual
diagnostic records that need to be cleared.

When a record is fixed, remove the matching entry from
`crates/shuck-cli/tests/testdata/corpus-metadata/s001.yaml` in the same change.
Resolved records should not remain as reviewed divergences.

## [ ] ShellCheck-only: status/return operand shuck marked safe (7)

- `bittorf__kalua__openwrt-addons__etc__kalua__watch:2096:10-15` `$good`
- `rvm__rvm__scripts__functions__manage__base_fetch:223:18-25` `$result`
- `rvm__rvm__scripts__functions__manage__base_fetch:243:24-31` `$result`
- `rvm__rvm__scripts__functions__manage__base_fetch:251:18-25` `$result`
- `rvm__rvm__scripts__functions__manage__macruby:145:14-21` `$result`
- `rvm__rvm__scripts__functions__requirements__osx_brew:485:55-74` `$homebrew_installer`
- `rvm__rvm__scripts__functions__requirements__osx_brew:486:32-51` `$homebrew_installer`

## [ ] ShellCheck-only: embedded path/URL/composite word (4)

- `233boy__v2ray__src__core.sh:1254:39-43` `$net`
- `juewuy__ShellCrash__scripts__menus__9_upgrade.sh:651:70-80` `${project}`
- `juewuy__ShellCrash__scripts__menus__9_upgrade.sh:867:52-62` `${db_type}`
- `rvm__rvm__scripts__functions__requirements__osx_brew:491:35-51` `${homebrew_repo}`

## [ ] ShellCheck-only: plain command argument (4)

- `RetroPie__RetroPie-Setup__scriptmodules__supplementary__runcommand__runcommand.sh:235:32-38` `$group`
- `community-scripts__ProxmoxVE__vm__haos-vm.sh:636:23-35` `${DISK_SIZE}`
- `community-scripts__ProxmoxVE__vm__mikrotik-routeros.sh:655:25-37` `${DISK_SIZE}`
- `gentoo__gentoo__eclass__tests__toolchain.sh:155:7-13` `${ret}`

## [ ] ShellCheck-only: test/probe command operand (2)

- `rvm__rvm__binscripts__rvm-installer:89:24-48` `${rvm_tar_command:-gtar}`
- `tteck__Proxmox__vm__nextcloud-vm.sh:210:96-99` `$HN`

## [ ] Shuck-only: embedded safe literal/composite word (1)

- `bittorf__kalua__openwrt-addons__etc__init.d__override_uci_vars:418:29-31` `$i`

## [ ] Shuck-only: simple command argument ShellCheck suppresses (8)

- `bats-core__bats-core__libexec__bats-core__bats-format-pretty:67:16-34` `$count_column_left`
- `bats-core__bats-core__libexec__bats-core__bats-format-pretty:78:13-32` `$line_backoff_count`
- `bittorf__kalua__openwrt-monitoring__ping_counter.sh:110:7-15` `$fileage`
- `gentoo__gentoo__eclass__tests__toolchain-funcs.sh:61:6-12` `${ret}`
- `ko1nksm__shellspec__lib__general.sh:442:15-39` `$shellspec_readfile_data`
- `ko1nksm__shellspec__lib__libexec__shellspec.sh:94:37-39` `$c`
- `masonr__yet-another-bench-script__yabs.sh:991:12-19` `$GB_URL`
- `pi-hole__pi-hole__automated install__basic-install.sh:1833:21-39` `${webInterfaceDir}`

## [ ] Shuck-only: command-substitution initializer argument (3 remaining)

- `alexanderepstein__Bash-Snippets__bak2dvd__bak2dvd:245:36-43` `$tarpid`
- `alexanderepstein__Bash-Snippets__bak2dvd__bak2dvd:259:36-43` `$tarpid`
- `swoodford__aws__vpc-sg-import-rules-cloudflare.sh:281:101-106` `$PORT`

## [ ] Shuck-only: status/return/exit operand (3)

- `bittorf__kalua__openwrt-addons__etc__profile.d__kalua.sh:11:33-36` `$rc`
- `rvm__rvm__scripts__functions__manage__macruby:23:10-21` `${__result}`
- `v1s1t0r1sh3r3__airgeddon__airgeddon.sh:16442:11-23` `${exit_code}`

## [ ] Shuck-only: arithmetic/parameter-operator form (2)

- `bittorf__kalua__openwrt-monitoring__meshrdf_generate_table.sh:3374:6-27` `${inet_offer_down:-0}`
- `dehydrated-io__dehydrated__dehydrated:1077:85-109` `${account_key_sigalgo:2}`
