#!/bin/bash

# Invalid: different string literals cannot all match the same value.
[[ "$x" != a ]] || [[ "$x" != b ]]

# Invalid: the tested value may appear on either side of the comparison.
[[ a != "$x" ]] || [[ "$x" != b ]]

# Invalid: numeric not-equal tests have the same OR-chain shape.
[ "$n" -ne 1 ] || [ 2 -ne "$n" ]

# Invalid: bracket numeric tests compare decimal integers.
[ "$n" -ne 010 ] || [ "$n" -ne 8 ]

# Invalid: conditional numeric tests compare shell arithmetic integers.
[[ "$n" -ne 010 ]] || [[ "$n" -ne 10 ]]
[[ "$n" -ne 64#@ ]] || [[ "$n" -ne 63 ]]

# Invalid: quoted conditional globs are fixed string literals.
[[ "$x" != "a*" ]] || [[ "$x" != "b*" ]]
[[ "$x" != 'a*' ]] || [[ "$x" != 'b*' ]]

# Invalid: the later conflicting test can appear after another fallback.
[[ a != "$x" ]] || maybe || [[ b != "$x" ]]

# Invalid: each later conflicting comparison is reported.
[[ "$x" != a ]] || [[ "$x" != b ]] || [[ "$x" != c ]]

# Valid: positive comparisons can all fail.
[[ "$x" == a ]] || [[ "$x" == b ]]

# Valid: the tests check different values.
[[ "$x" != a ]] || [[ "$y" != b ]]

# Valid: repeating the same negative comparison is not this tautology.
[[ "$x" != a ]] || [[ "$x" != a ]]

# Valid: bracket numeric tests use decimal interpretation.
[ "$n" -ne 010 ] || [ "$n" -ne 10 ]

# Valid: conditional numeric tests use shell arithmetic interpretation.
[[ "$n" -ne 010 ]] || [[ "$n" -ne 8 ]]
[[ "$n" -ne 36#Z ]] || [[ "$n" -ne 35 ]]
[[ "$n" -ne 64#_ ]] || [[ "$n" -ne 63 ]]

# Valid: numeric chains with invalid literal operands can fail with an error.
[ "$n" -ne x ] || [ "$n" -ne y ]
[[ "$n" -ne x ]] || [[ "$n" -ne y ]]
[[ "$n" -ne 8#9 ]] || [[ "$n" -ne 10 ]]

# Valid: the literal-looking sides are runtime-sensitive patterns.
[[ "$x" != a* ]] || [[ "$x" != b* ]]

# Valid: quote and expansion spelling changes are not assumed equivalent.
[[ a != "$x" ]] || [[ b != $x ]]

# Valid: the test command is left alone for compatibility.
test "$x" != a || test "$x" != b
