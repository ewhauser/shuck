# Manual Neovim Black-Box Tests

This directory holds the Neovim-backed LSP smoke harness for `shuck server`.

It exercises a real headless Neovim client talking to a real `shuck server`
process over stdio, with a feature-organized fixture workspace staged into a
temporary directory for each run.

The same harness powers the GitHub Actions job named `LSP Integration Tests`.

## Prerequisites

Enter the repo dev shell so the expected runtimes are on `PATH`:

```bash
nix --extra-experimental-features 'nix-command flakes' develop
```

## Run

Run the full smoke suite:

```bash
python3 crates/shuck-server/tests/manual/run_neovim_blackbox.py
```

Run a single scenario:

```bash
python3 crates/shuck-server/tests/manual/run_neovim_blackbox.py --case diagnostics/open_edit
python3 crates/shuck-server/tests/manual/run_neovim_blackbox.py --case code_actions/quick_fix
```

Available scenarios:

- `diagnostics/open_edit`
- `hover/rule_directive`
- `code_actions/quick_fix`
- `code_actions/fix_all`
- `formatting/request_round_trip`
- `configuration/reload_workspace_config`

The runner builds `target/debug/shuck`, stages `fixtures/workspace/` into a
temporary workspace, launches `nvim --headless`, and exits non-zero if either
the Neovim-side assertions or the server transport fail.

The formatting scenario currently verifies request round-trip behavior only.
`shuck-server` does not advertise formatting capabilities yet, and
`shuck-formatter` is still a no-op stub, so the expected result today is an
empty edit list rather than rewritten buffer contents.
