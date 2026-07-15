# shuck-wasm

WebAssembly bindings for the [Shuck](https://github.com/ewhauser/shuck) shell
script linter and formatter.

The npm package is built for wasm-aware bundlers such as webpack, Rollup, or
esbuild. It can be bundled into Node.js editor extensions and browser-hosted
editors without installing the native `shuck` executable.

```sh
npm install shuck-wasm
```

```ts
import { format, lint, version } from "shuck-wasm";

const diagnostics = lint("echo $name\n", {
  filename: "script.bash",
  select: ["ALL"],
});

for (const diagnostic of diagnostics) {
  console.log(diagnostic.code, diagnostic.range, diagnostic.message);
}

const formatted = format("hello(){\necho hi\n}\n", {
  shell: "bash",
  indentStyle: "space",
  indentWidth: 2,
});

console.log(`shuck ${version()}`);
```

Diagnostic and fix ranges use zero-based UTF-16 positions, matching VS Code and
the Language Server Protocol default. Linting is source-only: `filename` is a
logical path used for shell inference and policy context, and is never read from
the filesystem.

The package exposes source linting and formatting. Project discovery,
configuration-file loading, embedded shell extraction, source-file resolution,
and the stdio language server remain features of the native Shuck CLI.
