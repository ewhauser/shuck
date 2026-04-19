#!/bin/sh
# Should trigger: adjacent bracket text after an unbraced variable.
printf '%s\n' "$name[0]"

# Should trigger: bracket expressions after an unbraced variable in a pattern.
pattern="^$key[[:space:]]*$"

# Should trigger: literal prefixes do not change the ambiguity.
entry=game$game[0]

# Should trigger: command names are still ordinary words here.
$cmd[0] arg

# Should not trigger: braces make the boundary explicit.
printf '%s\n' "${name}[0]"

# Should not trigger: quote boundaries break the adjacency.
printf '%s\n' "$name""[0]" "$name"'[0]'

# Should not trigger: escaped brackets stay literal.
printf '%s\n' "$name\[0]"

# Should not trigger: positional parameters are not part of this rule.
printf '%s\n' "$1[0]"
