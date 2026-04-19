#!/bin/bash

# Invalid: lexical `<` with a dotted version literal is not numeric ordering.
[[ $ver < 1.27 ]]

# Invalid: the same issue applies when the dotted version is on the left.
[[ 1.2 < $ver ]]

# Invalid: comparing two dotted numeric literals still uses lexical ordering.
[[ 1.2.3 < 2.0 ]]

# Invalid: lexical `>` still compares version-like values as strings.
[[ $ver > 1.27 ]]

# Invalid: nested lexical version comparisons inside logical conditions still compare strings.
[[ $ver < 1.27 && -n $x ]]

# Valid: integer-only comparisons belong to the generic numeric/string rule family.
[[ $count < 10 ]]

# Valid: plain string ordering is outside this version-specific rule.
[[ foo < bar ]]
[[ $tag < v1.2 ]]
