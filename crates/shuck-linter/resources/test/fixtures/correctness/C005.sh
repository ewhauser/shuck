#!/bin/bash

# Invalid: single-quoted variable reference stays literal
echo '$HOME'

# Invalid: parameter expansion is also literal inside single quotes
printf '%s\n' '${value:-fallback}'

# Invalid: command substitution text inside single quotes is not executed
msg='$(pwd)'

# Invalid: backticks inside single quotes are also literal
echo '`pwd`'

# Invalid: sed should still warn for variable-like single-quoted text
sed -n '$pattern'

# Invalid: find -exec should use the effective subcommand
find . -exec echo '$1' {} +

# Invalid: git subcommands should still warn when not exempt
git '$a'

# Valid: double-quoted variable references still expand
echo "$HOME"

# Valid: ordinary single-quoted text is fine
echo 'hello world'

# Valid: escaped dollar in double quotes is explicit literal text
echo "\$HOME"

# Valid: ShellCheck does not flag special parameters for SC2016
echo '$$'
echo '$?'
echo '$#'
echo '$@'
echo '$*'
echo '$!'
echo '$-'

# Valid: sed-specific patterns are exempt
sed 's/foo$/bar/'
sed -n '$p'
sed '${/lol/d}'

# Valid: awk programs are exempt
awk '{print $1}'
busybox awk '{print $1}'
find . -exec awk '{print $1}' {} \;

# Valid: command-specific exemptions
trap 'echo $SECONDS' EXIT
eval 'echo $1'
alias hosts='sudo $EDITOR /etc/hosts'
git filter-branch 'test $GIT_COMMIT'
rename 's/(.)a/$1/g' *
jq '$__loc__'
command jq '$__loc__'
exec jq '$__loc__'
exec -c -a foo jq '$__loc__'

# Valid: prompt assignments are exempt
PS1='$PWD \\$ '
export PS4='$PWD'

# Valid: -v test operands are exempt
[ -v 'bar[$foo]' ]
