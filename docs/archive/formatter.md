# Archived formatter docs

This file preserves the user-facing formatter docs that were removed from the
README and website while the formatter stays gated behind
`SHUCK_EXPERIMENTAL`.

Do not link this from public docs until the formatter is ready to be promoted
again.

## CLI usage

```sh
# Format files in-place
shuck format .

# Check formatting without modifying files (exit 1 if changes are needed)
shuck format --check .

# Show diffs instead of writing files
shuck format --diff .

# Format with specific options
shuck format --indent-style space --indent-width 4 .

# Minify (compact form, strip comments)
shuck format --minify script.sh
```

## Config file

```toml
[format]
indent-style = "space"     # tab | space
indent-width = 4           # 1-255, used when indent-style = "space"
binary-next-line = false   # place binary operators on the next line
switch-case-indent = false # indent case branch bodies
space-redirects = false    # spaces around redirect operators
keep-padding = false       # preserve original source padding
function-next-line = false # opening brace on its own line
never-split = false        # compact single-line layouts
```

Formatter dialect is auto-discovered from the filename or shebang. Use
`shuck format --dialect <shell>` when you need an explicit override,
especially for stdin input.

## Website copy

- Built-in formatter with configurable indentation, operator placement, and
  layout
- `shuck format .`
- `shuck format --check .`
