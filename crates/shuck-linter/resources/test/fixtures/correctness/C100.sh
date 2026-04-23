#!/bin/bash

# shellcheck disable=2034

# Invalid: plain unindexed array references.
arr=(one two)
declare -A map=([key]=value)
read -ra read_items
mapfile map_items
x="$BASH_SOURCE"
y="${BASH_SOURCE}"
printf '%s\n' $arr "${arr}" pre${arr}post "$map" "$read_items" "$map_items"
printf '%s\n' "$BASH_SOURCE" "${BASH_SOURCE}"
source "$(dirname "$BASH_SOURCE")/helper.bash"
if [[ "$BASH_SOURCE" == "main.bash" ]]; then :; fi
for item in "$BASH_SOURCE"; do
  :
done
cat <<EOF
$arr
${arr}
EOF

# Invalid: unquoted BASH_SOURCE forms are still plain array references.
x=$BASH_SOURCE
y=${BASH_SOURCE}

# Valid: indexed and array-selector forms are explicit.
z="${BASH_SOURCE[0]}"
q="${BASH_SOURCE[@]}"
r="${BASH_SOURCE[*]}"
m="${arr[0]}"
n="${arr[@]}"
o="${arr[*]}"

# Valid: operation forms are outside this rule.
s="${BASH_SOURCE%/*}"
t="${BASH_SOURCE:-fallback}"
p="${arr%one}"
fallback="${arr:-fallback}"
