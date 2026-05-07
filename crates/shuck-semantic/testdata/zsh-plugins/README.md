# Zsh Plugin Fixtures

These fixtures model small, real-world zsh plugin layouts for source-closure
and plugin-manager tests. Keep each case focused: include the user startup file,
the relevant framework/plugin-manager root, and any standalone plugin repositories
that should be resolved automatically.

The files are intentionally tiny. They should preserve the loading shape that
Shuck needs to understand without copying full third-party plugins into tests.

