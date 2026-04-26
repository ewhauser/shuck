# S001 Reviewed Divergence Bugs

This file classifies the current reviewed S001/SC2086 large-corpus divergences.
The source of truth was a fresh `make large-corpus-report SHUCK_LARGE_CORPUS_RULES=S001`
run with ShellCheck 0.11.0.

Current summary from `target/large-corpus-report/latest.log`:

- `implementation_diffs=0`
- `mapping_issues=0`
- `reviewed_divergences=61`
- Individual reviewed records classified below: 97 total (`shellcheck-only=23`, `shuck-only=74`).

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

## [ ] ShellCheck-only: embedded path/URL/composite word (6)

- `233boy__v2ray__src__core.sh:1254:39-43` `$net`
- `juewuy__ShellCrash__scripts__menus__9_upgrade.sh:651:70-80` `${project}`
- `juewuy__ShellCrash__scripts__menus__9_upgrade.sh:867:52-62` `${db_type}`
- `megastep__makeself__test__variabletest:15:56-60` `${1}`
- `rvm__rvm__scripts__functions__requirements__osx_brew:491:35-51` `${homebrew_repo}`
- `termux__termux-packages__packages__lazygit__build.sh:26:30-50` `${SOURCE_DATE_EPOCH}`

## [ ] ShellCheck-only: plain command argument (5)

- `RetroPie__RetroPie-Setup__scriptmodules__supplementary__runcommand__runcommand.sh:235:32-38` `$group`
- `community-scripts__ProxmoxVE__vm__haos-vm.sh:636:23-35` `${DISK_SIZE}`
- `community-scripts__ProxmoxVE__vm__mikrotik-routeros.sh:655:25-37` `${DISK_SIZE}`
- `gentoo__gentoo__eclass__tests__toolchain.sh:155:7-13` `${ret}`
- `nvm-sh__nvm__test__slow__nvm_get_latest__nvm_get_latest:26:10-14` `$URL`

## [ ] ShellCheck-only: nested command substitution / here-string argument (3)

- `bittorf__kalua__openwrt-monitoring__meshrdf_generate_map.sh:263:70-75` `$LINE`
- `bittorf__kalua__openwrt-monitoring__meshrdf_generate_netjson.sh:577:70-75` `$LINE`
- `google__oss-fuzz__projects__threetenbp__build.sh:57:37-47` `$JAVA_HOME`

## [ ] ShellCheck-only: test/probe command operand (2)

- `rvm__rvm__binscripts__rvm-installer:89:24-48` `${rvm_tar_command:-gtar}`
- `tteck__Proxmox__vm__nextcloud-vm.sh:210:96-99` `$HN`

## [ ] Shuck-only: embedded safe literal/composite word (38)

- `bittorf__kalua__openwrt-addons__etc__init.d__override_uci_vars:418:29-31` `$i`
- `community-scripts__ProxmoxVE__vm__archlinux-vm.sh:547:31-44` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__debian-13-vm.sh:615:27-36` `${FORMAT}`
- `community-scripts__ProxmoxVE__vm__debian-13-vm.sh:616:25-38` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__debian-13-vm.sh:622:27-36` `${FORMAT}`
- `community-scripts__ProxmoxVE__vm__debian-13-vm.sh:623:25-38` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__debian-vm.sh:556:27-36` `${FORMAT}`
- `community-scripts__ProxmoxVE__vm__debian-vm.sh:557:25-38` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__debian-vm.sh:563:27-36` `${FORMAT}`
- `community-scripts__ProxmoxVE__vm__debian-vm.sh:564:25-38` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__nextcloud-vm.sh:542:25-34` `${FORMAT}`
- `community-scripts__ProxmoxVE__vm__nextcloud-vm.sh:543:23-36` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__nextcloud-vm.sh:544:23-36` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__opnsense-vm.sh:744:25-34` `${FORMAT}`
- `community-scripts__ProxmoxVE__vm__opnsense-vm.sh:745:23-36` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__owncloud-vm.sh:555:25-34` `${FORMAT}`
- `community-scripts__ProxmoxVE__vm__owncloud-vm.sh:556:23-36` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__owncloud-vm.sh:557:23-36` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__ubuntu2204-vm.sh:537:25-34` `${FORMAT}`
- `community-scripts__ProxmoxVE__vm__ubuntu2204-vm.sh:538:23-36` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__ubuntu2404-vm.sh:539:25-34` `${FORMAT}`
- `community-scripts__ProxmoxVE__vm__ubuntu2404-vm.sh:540:23-36` `${DISK_CACHE}`
- `community-scripts__ProxmoxVE__vm__ubuntu2504-vm.sh:538:25-34` `${FORMAT}`
- `community-scripts__ProxmoxVE__vm__ubuntu2504-vm.sh:539:23-36` `${DISK_CACHE}`
- `rvm__rvm__scripts__functions__requirements__rvm_pkg:17:17-35` `${_read_char_flag}`
- `rvm__rvm__scripts__functions__requirements__unknown:74:17-35` `${_read_char_flag}`
- `tteck__Proxmox__vm__debian-vm.sh:409:25-34` `${FORMAT}`
- `tteck__Proxmox__vm__debian-vm.sh:410:23-36` `${DISK_CACHE}`
- `tteck__Proxmox__vm__haos-vm.sh:451:25-34` `${FORMAT}`
- `tteck__Proxmox__vm__haos-vm.sh:452:23-36` `${DISK_CACHE}`
- `tteck__Proxmox__vm__nextcloud-vm.sh:408:25-34` `${FORMAT}`
- `tteck__Proxmox__vm__nextcloud-vm.sh:409:23-36` `${DISK_CACHE}`
- `tteck__Proxmox__vm__owncloud-vm.sh:408:25-34` `${FORMAT}`
- `tteck__Proxmox__vm__owncloud-vm.sh:409:23-36` `${DISK_CACHE}`
- `tteck__Proxmox__vm__ubuntu2204-vm.sh:409:25-34` `${FORMAT}`
- `tteck__Proxmox__vm__ubuntu2204-vm.sh:410:23-36` `${DISK_CACHE}`
- `tteck__Proxmox__vm__ubuntu2404-vm.sh:399:25-34` `${FORMAT}`
- `tteck__Proxmox__vm__ubuntu2404-vm.sh:400:23-36` `${DISK_CACHE}`

## [ ] Shuck-only: simple command argument ShellCheck suppresses (18)

- `SlackBuildsOrg__slackbuilds__system__sboui__doinst.sh:19:8-12` `$OLD`
- `SlackBuildsOrg__slackbuilds__system__sboui__doinst.sh:19:13-17` `$NEW`
- `awslabs__git-secrets__git-secrets:124:5-17` `${RECURSIVE}`
- `bats-core__bats-core__libexec__bats-core__bats-format-pretty:67:16-34` `$count_column_left`
- `bats-core__bats-core__libexec__bats-core__bats-format-pretty:78:13-32` `$line_backoff_count`
- `bittorf__kalua__openwrt-monitoring__ping_counter.sh:110:7-15` `$fileage`
- `gentoo__gentoo__eclass__tests__toolchain-funcs.sh:61:6-12` `${ret}`
- `google__oss-fuzz__projects__threetenbp__build.sh:56:89-99` `LD_LIBRARY_PATH`
- `juewuy__ShellCrash__scripts__menus__2_settings.sh:360:41-51` `$redir_mod`
- `juewuy__ShellCrash__scripts__menus__2_settings.sh:368:41-51` `$redir_mod`
- `juewuy__ShellCrash__scripts__menus__2_settings.sh:377:41-51` `$redir_mod`
- `ko1nksm__shellspec__lib__general.sh:442:15-39` `$shellspec_readfile_data`
- `ko1nksm__shellspec__lib__libexec__shellspec.sh:94:37-39` `$c`
- `kward__shunit2__.githooks__generic:30:11-22` `${basename}`
- `masonr__yet-another-bench-script__yabs.sh:991:12-19` `$GB_URL`
- `pi-hole__pi-hole__automated install__basic-install.sh:1833:21-39` `${webInterfaceDir}`
- `scop__bash-completion__completions-core__geoiplookup.bash:29:31-36` `$ipvx`
- `termux__termux-packages__packages__lazygit__build.sh:25:41-61` `-ldflags`

## [ ] Shuck-only: command-substitution initializer argument (4 remaining)

- `alexanderepstein__Bash-Snippets__bak2dvd__bak2dvd:245:36-43` `$tarpid`
- `alexanderepstein__Bash-Snippets__bak2dvd__bak2dvd:259:36-43` `$tarpid`
- `swoodford__aws__vpc-sg-import-rules-cloudflare.sh:281:101-106` `$PORT`
- `swoodford__aws__wafv2-web-acl-pingdom.sh:196:40-49` `$WAFSCOPE`

## [ ] Shuck-only: numeric/test operand (7)

- `bittorf__kalua__openwrt-addons__etc__kalua__watch:362:40-58` `${overall:-$count}`
- `bittorf__kalua__openwrt-monitoring__send_sms.sh:145:10-19` `${pos:-0}`
- `nvm-sh__nvm__nvm.sh:3785:16-25` `$nosource`
- `pi-hole__pi-hole__advanced__Scripts__piholeCheckout.sh:191:14-30` `$download_status`
- `pi-hole__pi-hole__advanced__Scripts__piholeCheckout.sh:215:18-34` `$download_status`
- `pi-hole__pi-hole__advanced__Scripts__piholeCheckout.sh:221:20-36` `$download_status`
- `super-linter__super-linter__test__run-super-linter-tests.sh:724:36-57` `${EXPECTED_EXIT_CODE}`

## [ ] Shuck-only: status/return/exit operand (7)

- `bittorf__kalua__openwrt-addons__etc__profile.d__kalua.sh:11:33-36` `$rc`
- `rvm__rvm__scripts__extras__chruby.sh:31:10-21` `${__result}`
- `rvm__rvm__scripts__functions__build_requirements:160:10-29` `${__summary_status}`
- `rvm__rvm__scripts__functions__manage__base_fetch:45:12-24` `${result:-0}`
- `rvm__rvm__scripts__functions__manage__macruby:23:10-21` `${__result}`
- `rvm__rvm__scripts__functions__requirements__openbsd:35:12-23` `${__result}`
- `v1s1t0r1sh3r3__airgeddon__airgeddon.sh:16442:11-23` `${exit_code}`

## [ ] Shuck-only: arithmetic/parameter-operator form (2)

- `bittorf__kalua__openwrt-monitoring__meshrdf_generate_table.sh:3374:6-27` `${inet_offer_down:-0}`
- `dehydrated-io__dehydrated__dehydrated:1077:85-109` `${account_key_sigalgo:2}`
