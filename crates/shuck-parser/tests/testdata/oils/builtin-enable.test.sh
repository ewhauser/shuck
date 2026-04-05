## oils_failures_allowed: 0
## compare_shells: bash

#### enable -n resolves regular builtins through PATH and disables builtin dispatch
PATH=/usr/bin:/bin
enable -n printf
command -v printf | egrep -o '/printf$'
type -t printf
builtin printf hi 2>err.txt
echo status=$?
grep -o 'not a shell builtin' err.txt
enable printf
command -v printf
type -t printf

## STDOUT:
/printf
file
status=1
not a shell builtin
printf
builtin
## END

#### enable -n can disable POSIX special builtin lookup
set -o posix
enable -n eval
eval() { echo func:$1; }
type -t eval
eval hi
enable eval
type -t eval

## STDOUT:
function
func:hi
builtin
## END

#### enable affects compgen and help views
PATH=/usr/bin:/bin
enable -n eval
compgen -A builtin ev
echo --
compgen -A helptopic ev
echo --
compgen -A command ev | grep '^eval$' || echo none
echo --
enable -n printf
compgen -A command printf | sort -u
echo --
help | grep -o '\*printf .*'
echo --
help -s printf

## STDOUT:
eval
--
eval
--
none
--
printf
--
*printf [-v var] format [arguments]
--
printf: printf [-v var] format [arguments]
## END
