# 024: WebAssembly npm Package

## Status

Implemented

## Summary

Shuck publishes a `shuck-wasm` npm package containing the parser, linter, and
formatter compiled to WebAssembly. The package gives bundled Node.js extensions
and browser-hosted editors the same in-process analysis primitives without
requiring the native `shuck` executable.

## Motivation

The native CLI and stdio language server work well when an editor can launch a
local process. They do not work in browser-only environments such as VS Code for
the Web, and requiring a separately installed binary complicates otherwise
self-contained Node.js editor extensions.

The package must therefore:

- run without filesystem, process, network, or thread access;
- expose editor-ready UTF-16 ranges and rule fixes;
- infer a shell from source text and a logical filename while allowing an
  explicit override;
- keep its version synchronized with the rest of the workspace; and
- be built and published from a tagged Shuck release with provenance.

## Design

### Workspace crate and npm artifact

`crates/shuck-wasm` is a non-crates.io workspace crate with `cdylib` and `rlib`
outputs. It depends directly on the parser, linter, indexer, and formatter
libraries rather than the CLI or language server, avoiding native-only file
discovery, caching, process management, and stdio transport.

`wasm-pack` builds the crate for `wasm32-unknown-unknown` using its `bundler`
target. The resulting package is named `shuck-wasm`. Its generated JavaScript
imports the `.wasm` module, which makes it suitable for extension toolchains
that bundle npm dependencies for Node.js or the browser.

The generated package is a release artifact and is not committed. The Cargo
workspace version is its only version source.

### JavaScript API

The initial API is synchronous after the bundler loads the module:

```ts
export function version(): string;
export function lint(source: string, options?: LintOptions): Diagnostic[];
export function format(source: string, options?: FormatOptions): string;
```

`LintOptions` supports a logical `filename`, a shell override, and `select` and
`ignore` rule selectors. An omitted `select` uses Shuck's default rule set; an
explicit empty array selects no lint rules. Syntax diagnostics are still
returned for malformed input.

Each diagnostic contains its code, message, severity, range, and optional fix.
Ranges use zero-based UTF-16 code-unit positions, matching the default Language
Server Protocol and VS Code position encoding. Fixes contain replacement edits
in the same coordinate system and retain Shuck's safe/unsafe applicability.

`FormatOptions` exposes the source-only formatter controls that do not require
project configuration or filesystem access. Formatting returns the original
source when no change is necessary and throws a JavaScript `Error` for invalid
options or malformed shell input.

### Runtime boundaries

WASM lint requests always disable source-closure resolution. A logical filename
is used only for dialect inference, per-file policy context, and diagnostic
behavior; the runtime never reads that path. Configuration-file discovery,
embedded host-file extraction, multi-file analysis, and LSP transport remain
native responsibilities.

### Release automation

CI builds a Node.js-targeted smoke package, executes linting and formatting
through the generated JavaScript, and builds the exact bundler artifact intended
for npm.

When a GitHub release is published, a protected release job checks out the tag,
rebuilds the bundler package, verifies that the tag and package versions match,
and runs `npm publish`. npm trusted publishing supplies short-lived OIDC
credentials and provenance; no long-lived npm token is stored in the repository.

## Alternatives Considered

### Package native binaries through npm

Platform-specific binaries would work for desktop Node.js extensions but still
could not run in browser-hosted editors. They would also duplicate the existing
native release matrix and launcher selection logic.

### Compile the CLI or stdio language server directly

Those crates intentionally depend on filesystem discovery, caching, process
execution, and native transport. Compiling the library pipeline instead keeps
the WASM boundary small and gives editors direct access to structured results.

### Publish separate browser and Node.js packages

Separate packages would duplicate the WebAssembly payload and create two
versioned public surfaces. The bundler target covers the extension use case in
both environments; an unbundled Node.js entrypoint can be added later if a
concrete consumer requires one.

### Commit wasm-pack output

Generated JavaScript and binary output are release artifacts. Rebuilding them in
CI from the tagged source gives npm provenance a direct, auditable build path and
avoids noisy binary changes in the repository.

## Security Considerations

The API accepts untrusted source and options, so it retains the parser's depth
and fuel limits and performs no ambient I/O. Options are strictly deserialized;
unknown fields and rule selectors fail with a JavaScript error. Publishing uses
GitHub-hosted runners, minimal permissions, a protected `release` environment,
and npm OIDC trusted publishing.

## Verification

Run:

```sh
cargo test -p shuck-wasm
make test-wasm
make build-wasm
npm pack --dry-run ./target/npm/shuck-wasm
```

`make test-wasm` builds a Node.js-targeted package and executes the public API.
CI additionally compiles and packs the bundler artifact. A release is complete
when the workflow publishes `shuck-wasm` at the same version as its `vX.Y.Z`
tag and npm records provenance for it.
