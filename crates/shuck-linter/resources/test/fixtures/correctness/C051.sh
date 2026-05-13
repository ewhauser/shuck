#!/bin/bash

# Invalid: both redirects write stdout.
: >/tmp/first >/tmp/second

# Invalid: both redirects write the same explicit descriptor.
: 2>/tmp/first 2>/tmp/second

# Invalid: input redirects also override each other.
cat </tmp/first </tmp/second

# Invalid: here-doc redirects share descriptor zero with file input.
cat <<EOF </tmp/input
body
EOF

# Invalid: combined stdout/stderr redirection overlaps stdout.
: &>/tmp/all >/tmp/stdout

# Invalid: two combined redirections overlap both stdout and stderr.
: &>/tmp/first &>/tmp/second

# Invalid: `>&word` redirects both stdout and stderr when the target is not numeric.
: >&/tmp/all 2>/tmp/stderr

# Invalid: fd-prefixed `>&word` redirects that descriptor.
: 2>&/tmp/stderr 2>/tmp/next

# Valid: stdout and stderr are different descriptors.
: >/tmp/stdout 2>/tmp/stderr

# Valid: read-write redirects are left alone.
: <>/tmp/state >/tmp/stdout

# Valid: descriptor duplication is left alone.
: 2>&1 2>/tmp/stderr

# Valid: closing a descriptor is left alone.
: >&- >/tmp/stdout

# Valid: brace-assigned descriptors are dynamic.
exec {fd}>/tmp/first {fd}>/tmp/second
