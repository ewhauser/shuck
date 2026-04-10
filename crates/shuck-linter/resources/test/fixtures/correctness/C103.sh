#!/bin/bash

# Invalid: `-o` alternatives are not grouped before an action.
find . -name perllocal.pod -o -name ".packlist" -print

# Invalid: same precedence issue on another branch pair.
find . -name '*.tmp' -o -name '*.bak' -delete

# Valid: grouped alternatives before applying action.
find . \( -name perllocal.pod -o -name ".packlist" \) -print

# Valid: explicit `-a` avoids implicit precedence traps.
find . -name '*.tmp' -o -name '*.bak' -a -print

# Valid: if earlier branches already include actions, this pattern is intentional.
find . -name '*.tmp' -print -o -name '*.bak' -print
