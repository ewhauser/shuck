# Manual Neovim Black-Box Tests

This directory holds the Neovim-backed LSP smoke harness for `shuck server`.

It exercises a real headless Neovim client talking to a real `shuck server`
process over stdio, with the fixture workspace staged into a temporary
directory for each run.

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
python3 crates/shuck-server/tests/manual/run_neovim_blackbox.py --case diagnostics
python3 crates/shuck-server/tests/manual/run_neovim_blackbox.py --case hover
```

The runner builds `target/debug/shuck`, stages `fixtures/project/` into a
temporary workspace, launches `nvim --headless`, and exits non-zero if either
the Neovim-side assertions or the server transport fail.
