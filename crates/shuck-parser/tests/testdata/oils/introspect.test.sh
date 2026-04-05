## compare_shells: bash

# Test call stack introspection.  There are a bunch of special variables
# defined here:
#
# https://www.gnu.org/software/bash/manual/html_node/Bash-Variables.html
# 
# - The shell function ${FUNCNAME[$i]} is defined in the file
#   ${BASH_SOURCE[$i]} and called from ${BASH_SOURCE[$i+1]}
#
# - ${BASH_LINENO[$i]} is the line number in the source file
#   (${BASH_SOURCE[$i+1]}) where ${FUNCNAME[$i]} was called (or
#   ${BASH_LINENO[$i-1]} if referenced within another shell function). 
#
# - For instance, ${FUNCNAME[$i]} was called from the file
#   ${BASH_SOURCE[$i+1]} at line number ${BASH_LINENO[$i]}. The caller builtin
#   displays the current call stack using this information. 
#
# So ${BASH_SOURCE[@]} doesn't line up with ${BASH_LINENO}.  But
# ${BASH_SOURCE[0]} does line up with $LINENO!
#
# Geez.
#
# In other words, BASH_SOURCE is about the DEFINITION.  While FUNCNAME and
# BASH_LINENO are about the CALL.


#### ${FUNCNAME[@]} array
g() {
  argv.sh "${FUNCNAME[@]}"
}
f() {
  argv.sh "${FUNCNAME[@]}"
  g
  argv.sh "${FUNCNAME[@]}"
}
f
## STDOUT: 
['f']
['g', 'f']
['f']
## END

#### FUNCNAME with source (scalar or array)
cd $REPO_ROOT

# Comments on bash quirk:
# https://github.com/oilshell/oil/pull/656#issuecomment-599162211

f() {
  . spec/testdata/echo-funcname.sh
}
g() {
  f
}

g
echo -----

. spec/testdata/echo-funcname.sh
echo -----

argv.sh "${FUNCNAME[@]}"

# Show bash inconsistency.  FUNCNAME doesn't behave like a normal array.
case $SH in 
  (bash)
    echo -----
    a=('A')
    argv.sh '  @' "${a[@]}"
    argv.sh '  0' "${a[0]}"
    argv.sh '${}' "${a}"
    argv.sh '  $' "$a"
    ;;
esac

## STDOUT:
['  @', 'source', 'f', 'g']
['  0', 'source']
['${}', 'source']
['  $', 'source']
-----
['  @', 'source']
['  0', 'source']
['${}', 'source']
['  $', 'source']
-----
[]
## END
## BUG bash STDOUT:
['  @', 'source', 'f', 'g']
['  0', 'source']
['${}', 'source']
['  $', 'source']
-----
['  @']
['  0', '']
['${}', '']
['  $', '']
-----
[]
-----
['  @', 'A']
['  0', 'A']
['${}', 'A']
['  $', 'A']
## END


#### BASH_SOURCE and BASH_LINENO scalar or array (e.g. for virtualenv)
cd $REPO_ROOT

# https://github.com/pypa/virtualenv/blob/master/virtualenv_embedded/activate.sh
# https://github.com/akinomyoga/ble.sh/blob/6f6c2e5/ble.pp#L374

argv.sh "$BASH_SOURCE"  # SimpleVarSub
argv.sh "${BASH_SOURCE}"  # BracedVarSub
argv.sh "$BASH_LINENO"  # SimpleVarSub
argv.sh "${BASH_LINENO}"  # BracedVarSub
argv.sh "$FUNCNAME"  # SimpleVarSub
argv.sh "${FUNCNAME}"  # BracedVarSub
echo __
source spec/testdata/bash-source-string.sh

## STDOUT:
['']
['']
['']
['']
['']
['']
__
['spec/testdata/bash-source-string.sh']
['spec/testdata/bash-source-string.sh']
['11']
['11']
____
['spec/testdata/bash-source-string2.sh']
['spec/testdata/bash-source-string2.sh']
['11']
['11']
## END


#### ${FUNCNAME} with prefix/suffix operators

check() {
  argv.sh "${#FUNCNAME}"
  argv.sh "${FUNCNAME::1}"
  argv.sh "${FUNCNAME:1}"
}
check
## STDOUT:
['5']
['c']
['heck']
## END

#### operators on FUNCNAME
check() {
  argv.sh "${FUNCNAME}"
  argv.sh "${#FUNCNAME}"
  argv.sh "${FUNCNAME::1}"
  argv.sh "${FUNCNAME:1}"
}
check
## status: 0
## STDOUT:
['check']
['5']
['c']
['heck']
## END

#### ${FUNCNAME} and "set -u" (OSH regression)
set -u
argv.sh "$FUNCNAME"
## status: 1
## stdout-json: ""

#### $((BASH_LINENO)) (scalar form in arith)
check() {
  echo $((BASH_LINENO))
}
check
## stdout: 4

#### ${BASH_SOURCE[@]} with source and function name
cd $REPO_ROOT

argv.sh "${BASH_SOURCE[@]}"
source spec/testdata/bash-source-simple.sh
f
## STDOUT: 
[]
['spec/testdata/bash-source-simple.sh']
['spec/testdata/bash-source-simple.sh']
## END

#### ${BASH_SOURCE[@]} with line numbers
cd $REPO_ROOT

$SH spec/testdata/bash-source.sh
## STDOUT: 
['begin F funcs', 'f', 'main']
['begin F files', 'spec/testdata/bash-source.sh', 'spec/testdata/bash-source.sh']
['begin F lines', '21', '0']
['G funcs', 'g', 'f', 'main']
['G files', 'spec/testdata/bash-source-2.sh', 'spec/testdata/bash-source.sh', 'spec/testdata/bash-source.sh']
['G lines', '15', '21', '0']
['end F funcs', 'f', 'main']
['end F', 'spec/testdata/bash-source.sh', 'spec/testdata/bash-source.sh']
['end F lines', '21', '0']
## END

#### ${BASH_LINENO[@]} is a stack of line numbers for function calls
# note: it's CALLS, not DEFINITIONS.
g() {
  argv.sh G "${BASH_LINENO[@]}"
}
f() {
  argv.sh 'begin F' "${BASH_LINENO[@]}"
  g  # line 6
  argv.sh 'end F' "${BASH_LINENO[@]}"
}
argv.sh ${BASH_LINENO[@]}
f  # line 9
## STDOUT: 
[]
['begin F', '10']
['G', '6', '10']
['end F', '10']
## END

#### caller builtin in nested functions
cat > "$TMP/caller-lib.sh" <<'EOF'
inner() {
  caller 0
  echo "status=$?"
  caller 1
  echo "status=$?"
  caller 2
  echo "status=$?"
}
outer() {
  inner
}
EOF
. "$TMP/caller-lib.sh"
outer

#### caller builtin with sourced file and top level
cat > "$TMP/caller-inner.sh" <<'EOF'
caller 0
echo "source-top=$?"
inner() {
  caller 0
  echo "status=$?"
  caller 1
  echo "status=$?"
}
outer() {
  inner
}
EOF
cat > "$TMP/caller-outer.sh" <<'EOF'
. "$TMP/caller-inner.sh"
wrapper() {
  outer
}
wrapper
EOF
. "$TMP/caller-outer.sh"
caller 0
echo "top=$?"
caller 9
echo "far=$?"

#### Locations with temp frame

cd $REPO_ROOT

$SH spec/testdata/bash-source-pushtemp.sh

## STDOUT:
F
G
STACK:spec/testdata/bash-source-pushtemp.sh:g:3
STACK:spec/testdata/bash-source-pushtemp.sh:f:19
STACK:spec/testdata/bash-source-pushtemp.sh:main:0
## END

#### Locations when sourcing

cd $REPO_ROOT

# like above test case, but we source

# bash location doesn't make sense:
# - It says 'source' happens at line 1 of bash-source-pushtemp.  Well I think
# - It really happens at line 2 of '-c' !    I guess that's to line up
#   with the 'main' frame

$SH -c 'true;
source spec/testdata/bash-source-pushtemp.sh'

## STDOUT:
F
G
STACK:spec/testdata/bash-source-pushtemp.sh:g:3
STACK:spec/testdata/bash-source-pushtemp.sh:f:19
STACK:spec/testdata/bash-source-pushtemp.sh:source:2
## END

#### Sourcing inside function grows the debug stack

cd $REPO_ROOT

$SH spec/testdata/bash-source-source.sh

## STDOUT:
F
G
STACK:spec/testdata/bash-source-pushtemp.sh:g:3
STACK:spec/testdata/bash-source-pushtemp.sh:f:19
STACK:spec/testdata/bash-source-pushtemp.sh:source:2
STACK:spec/testdata/bash-source-source.sh:mainfunc:6
STACK:spec/testdata/bash-source-source.sh:main2:10
STACK:spec/testdata/bash-source-source.sh:main1:13
STACK:spec/testdata/bash-source-source.sh:main:0
## END
