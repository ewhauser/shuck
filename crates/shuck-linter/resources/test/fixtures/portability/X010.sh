#!/bin/sh

# Should trigger: comma-list brace expansion in an argument
echo {a,b}

# Should trigger: sequence brace expansion with a suffix
printf '%s\n' file{1..3}.txt

# Should trigger: brace expansion in an assignment value
name=pre{left,right}post
printf '%s\n' "$name"

# Should not trigger: quoted brace syntax stays literal
printf '%s\n' "{a,b}" '{1..3}'

# Should not trigger: parameter-expansion replacement patterns are a different syntax
printf '%s\n' "${name/{a,b}/x}"
