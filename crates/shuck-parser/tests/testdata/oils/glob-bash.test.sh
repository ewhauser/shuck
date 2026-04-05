## compare_shells: bash

#### shopt -s nullglob
argv.sh _tmp/spec-tmp/*.nonexistent
shopt -s nullglob
argv.sh _tmp/spec-tmp/*.nonexistent
## STDOUT:
['_tmp/spec-tmp/*.nonexistent']
[]
## END
## N-I dash/mksh/ash STDOUT:
['_tmp/spec-tmp/*.nonexistent']
['_tmp/spec-tmp/*.nonexistent']
## END

#### shopt -s failglob in command context
argv.sh *.ZZ
shopt -s failglob
argv.sh *.ZZ  # nothing is printed, not []
echo status=$?
## STDOUT:
['*.ZZ']
status=1
## END
## N-I dash/mksh/ash STDOUT:
['*.ZZ']
['*.ZZ']
status=0
## END

#### shopt -s failglob in loop context
for x in *.ZZ; do echo $x; done
echo status=$?
shopt -s failglob
for x in *.ZZ; do echo $x; done
echo status=$?
## STDOUT:
*.ZZ
status=0
status=1
## END
## N-I dash/mksh/ash STDOUT:
*.ZZ
status=0
*.ZZ
status=0
## END

#### shopt -s failglob in array literal context
myarr=(*.ZZ)
echo "${myarr[@]}"
shopt -s failglob
myarr=(*.ZZ)
echo status=$?
## STDOUT:
*.ZZ
status=1
## END
## N-I mksh STDOUT:
*.ZZ
status=0
## END
## N-I dash/ash stdout-json: ""
## N-I dash/ash status: 2

#### shopt -s failglob exits properly in command context with set -e
set -e
argv.sh *.ZZ
shopt -s failglob
argv.sh *.ZZ
echo status=$?
## STDOUT:
['*.ZZ']
## END
## status: 1
## N-I dash/mksh/ash STDOUT:
['*.ZZ']
## END
## N-I dash/mksh/ash status: 127

#### shopt -s failglob exits properly in loop context with set -e
set -e
for x in *.ZZ; do echo $x; done
echo status=$?

shopt -s failglob
for x in *.ZZ; do echo $x; done
echo status=$?

## status: 1
## STDOUT:
*.ZZ
status=0
## END

## N-I dash/mksh/ash status: 127
## N-I dash/mksh/ash STDOUT:
*.ZZ
status=0
## END

#### shopt -s failglob behavior on single line with semicolon
# bash behaves differently when commands are separated by a semicolon than when
# separated by a newline. This behavior doesn't make sense or seem to be
# intentional, so osh does not mimic it.

shopt -s failglob
echo *.ZZ; echo status=$? # bash doesn't execute the second part!
echo *.ZZ
echo status=$? # bash executes this

## STDOUT:
status=1
## END

## OK osh STDOUT:
status=1
status=1
## END

## N-I dash/mksh/ash STDOUT:
*.ZZ
status=0
*.ZZ
status=0
## END

#### dotglob (bash option that no_dash_glob is roughly consistent with)
mkdir -p $TMP/dotglob
cd $TMP/dotglob
touch .foorc other

echo *
shopt -s dotglob
echo * | sort
## STDOUT:
other
.foorc other
## END
## N-I dash/mksh/ash STDOUT:
other
other
## END

#### shopt -s nocaseglob matches bracket patterns
mkdir -p $TMP/nocaseglob
cd $TMP/nocaseglob
touch Alpha.TXT beta.txt Gamma.TxT

printf '%s\n' [a-z]*.txt | sort
shopt -s nocaseglob
printf '%s\n' [a-z]*.txt | sort

#### nocaseglob with nullglob and failglob
mkdir -p $TMP/nocaseglob-modes
cd $TMP/nocaseglob-modes
touch DATA.LOG

shopt -s nocaseglob nullglob
argv.sh *.log
rm DATA.LOG
argv.sh *.log

shopt -u nullglob
shopt -s failglob
echo *.log
echo status=$?
