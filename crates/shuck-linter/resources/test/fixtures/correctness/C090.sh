#!/bin/bash

# Invalid: `[ ]` comparisons do not treat the RHS as a glob pattern.
[ "$ARCH" == i?86 ]

# Invalid: the same problem applies to `=` and wildcard classes.
[ "$ARCH" = [[:digit:]] ]

# Invalid: `!=` still compares strings literally in `[ ]`.
[ "$ARCH" != *.x86 ]

# Valid: quoting the pattern makes it a plain string literal.
[ "$ARCH" == "i?86" ]

# Valid: escaping the wildcard makes it literal too.
[ "$ARCH" == i\?86 ]

# Valid: `[[ ]]` can do pattern matching directly.
[[ "$ARCH" == i?86 ]]
