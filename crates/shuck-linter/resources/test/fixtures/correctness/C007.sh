#!/bin/sh

# Invalid: raw find output is split on whitespace by xargs
find . -name '*.txt' | xargs rm

# Invalid: only enabling -0 on xargs is not enough
find . -type f | xargs -0 wc -l

# Invalid: only enabling -print0 on find is not enough
find "$pkg" -print0 | xargs rm

# Invalid: nested command substitutions should still be checked
summary=$(find . -type f | xargs wc -l)

# Valid: null-delimited handoff is safe
find . -name '*.txt' -print0 | xargs -0 rm

# Valid: non-find pipelines are outside this rule
printf '%s\n' ./a ./b | xargs rm
