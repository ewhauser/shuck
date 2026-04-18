#!/bin/bash

# Should trigger: a single dynamic echo argument feeds a simple sed rewrite.
echo $VALUE | sed 's/foo/bar/'
echo "$VALUE" | sed 's/foo/bar/g'
result=$(echo ${ITEMS[@]} | sed -e 's/foo/bar/2')

# Should not trigger: literals, affixes, flags, and non-matching sed forms.
echo literal | sed 's/foo/bar/'
echo prefix${VALUE}suffix | sed 's/foo/bar/'
echo $LEFT $RIGHT | sed 's/foo/bar/'
echo -n $VALUE | sed 's/foo/bar/'
echo $VALUE | sed -n 's/foo/bar/p'
echo $VALUE | sed --expression 's/foo/bar/'
echo $VALUE | sed -es/foo/bar/
echo $VALUE | sed 's/foo/bar/' | cat
printf '%s\n' "$VALUE" | sed 's/foo/bar/'
