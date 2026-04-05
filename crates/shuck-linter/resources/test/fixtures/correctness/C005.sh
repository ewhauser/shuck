#!/bin/sh

# Invalid: single-quoted variable reference stays literal
echo '$HOME'

# Invalid: parameter expansion is also literal inside single quotes
printf '%s\n' '${value:-fallback}'

# Invalid: command substitution text inside single quotes is not executed
msg='$(pwd)'

# Valid: double-quoted variable references still expand
echo "$HOME"

# Valid: ordinary single-quoted text is fine
echo 'hello world'

# Valid: escaped dollar in double quotes is explicit literal text
echo "\$HOME"
