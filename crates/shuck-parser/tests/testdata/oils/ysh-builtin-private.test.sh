## compare_shells: bash
## oils_failures_allowed: 1

#### type and command builtin don't find private sleep

remove-path() { sed 's;/.*/;;'; }

type -t sleep
type sleep | remove-path
echo

# this is meant to find the "first word"
type -a sleep | remove-path | uniq
echo

command -v sleep | remove-path

## STDOUT:
file
sleep is sleep

sleep is sleep

sleep
## END

#### builtin sleep behaves like external sleep
case $SH in
  *osh) prefix='builtin' ;;
  *) prefix='' ;;
esac

$prefix sleep
if test "$?" != 0; then
  echo ok
fi

# This is different!  OSH is stricter
if false; then
$prefix sleep --
if test "$?" != 0; then
  echo ok
fi
fi

$prefix sleep -2
if test "$?" != 0; then
  echo ok
fi

$prefix sleep -- -2
if test "$?" != 0; then
  echo ok
fi

$prefix sleep zz
if test "$?" != 0; then
  echo ok
fi

$prefix sleep 0
echo status=$?

$prefix sleep -- 0
echo status=$?

$prefix sleep '0.0005'
echo status=$?

$prefix sleep '+0.0005'
echo status=$?

## STDOUT:
ok
ok
ok
ok
status=0
status=0
status=0
status=0
## END

#### builtin sleep usage errors
case $SH in bash) exit ;; esac

builtin sleep 0.5s
echo status=$?

builtin sleep 0.1 extra
echo status=$?

## STDOUT:
status=2
status=2
## END
## N-I bash STDOUT:
## END

#### sleep without prefix is still external

# should not work
builtin sleep --version
if test "$?" != '0'; then
  echo ok
fi

sleep --version | head -n 1 >& 2
echo status=$?

## STDOUT:
ok
status=0
## END

#### builtin cat 

case $SH in
  *osh) prefix='builtin' ;;
  *) prefix='' ;;
esac

seq 2 3 | $prefix cat
echo ---

# from file
#echo FOO > foo
#$prefix cat foo foo

## STDOUT:
2
3
---
## END

#### builtin cat usage
case $SH in
  *osh) prefix='builtin' ;;
  *) prefix='' ;;
esac

$prefix cat --bad >/dev/null
if test "$?" != 0; then
  echo ok
fi

$prefix cat -z
if test "$?" != 0; then
  echo ok
fi

seq 3 4 | $prefix cat --
echo status=$?

## STDOUT:
ok
ok
3
4
status=0
## END

#### builtin cat non nonexistent file
case $SH in
  *osh) prefix='builtin' ;;
  *) prefix='' ;;
esac

echo FOO > foo

$prefix cat foo nonexistent foo
echo status=$?

## STDOUT:
FOO
FOO
status=1
## END

#### builtin cat accept - for stdin
case $SH in
  *osh) prefix='builtin' ;;
  *) prefix='' ;;
esac

echo FOO > foo
seq 3 4 | $prefix cat foo - foo foo

echo ---

# second - is a no-op
seq 5 6 | $prefix cat - -

## STDOUT:
FOO
3
4
FOO
FOO
---
5
6
## END

#### builtin rm: usage, removing files
case $SH in
  *osh) prefix='builtin' ;;
  *) prefix='' ;;
esac

$prefix rm 
if test "$?" != 0; then echo ok; fi

$prefix rm --
if test "$?" != 0; then echo ok; fi

$prefix rm -- nonexistent
echo status=$?

touch foo bar
$prefix rm -- foo bar
echo status=$?

if test -f foo; then echo fail; fi
if test -f bar; then echo fail; fi

## STDOUT:
ok
ok
status=1
status=0
## END

#### builtin rm -f - ignores arguments that don't exist
case $SH in
  *osh) prefix='builtin' ;;
  *) prefix='' ;;
esac

touch foo bar
$prefix rm -- foo OOPS bar
echo status=$?

if test -f foo; then echo fail; fi
if test -f bar; then echo fail; fi

touch foo bar
$prefix rm -f -- foo OOPS bar
echo status=$?

if test -f foo; then echo fail; fi
if test -f bar; then echo fail; fi

## STDOUT:
status=1
status=0
## END

#### builtin rm -f - still fails when file can't be removed

mkdir read-only
touch read-only/stuck
chmod -w read-only
ls -l stuck

touch foo bar
$prefix rm -- read-only/stuck foo bar
echo status=$?

if test -f foo; then echo fail; fi
if test -f bar; then echo fail; fi

ls read-only/stuck

touch foo bar
$prefix rm -- read-only/stuck foo bar
echo status=$?

# Clean up for real
chmod +w read-only
$prefix rm -- read-only/stuck
echo status=$?

if test -f read-only/stuck; then echo fail; fi

## STDOUT:
status=1
read-only/stuck
status=1
status=0
## END

#### builtin rm -f allows empty arg list
case $SH in
  *osh) prefix='builtin' ;;
  *) prefix='' ;;
esac

$prefix rm -f
echo status=$?

## STDOUT:
status=0
## END

#### builtin rm always fails on directories (regardless of -f)
case $SH in
  *osh) prefix='builtin' ;;
  *) prefix='' ;;
esac

touch foo bar
mkdir -p tmp
$prefix rm -- foo tmp bar
echo status=$?

if test -f foo; then echo fail; fi
if test -f bar; then echo fail; fi

touch foo bar
mkdir -p tmp
$prefix rm -f -- foo tmp bar
echo status=$?

if test -f foo; then echo fail; fi
if test -f bar; then echo fail; fi

## STDOUT:
status=1
status=1
## END

#### builtin readlink
case $SH in bash) exit ;; esac

echo TODO

# turn this into a builtin
# does that mean any builtin can be externalized?
# - [ aka test is a good candiate
# - we have stubs from true/false

## STDOUT:
## END

## N-I bash STDOUT:
## END

#### compgen -A builtin doesn't find private builtins

compgen -A builtin slee
echo status=$?

## STDOUT:
status=1
## END
