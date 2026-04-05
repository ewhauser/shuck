## compare_shells: dash bash mksh zsh ash

#### Parsing shell words \r \v

# frontend/lexer_def.py has rules for this

tab=$(printf 'argv.sh -\t-')
cr=$(printf 'argv.sh -\r-')
vert=$(printf 'argv.sh -\v-')
ff=$(printf 'argv.sh -\f-')

$SH -c "$tab"
$SH -c "$cr"
$SH -c "$vert"
$SH -c "$ff"

## STDOUT:
['-', '-']
['-\r-']
['-\x0b-']
['-\x0c-']
## END

#### \r in arith expression is allowed by some shells, but not most!

arith=$(printf 'argv.sh $(( 1 +\n2))')
arith_cr=$(printf 'argv.sh $(( 1 +\r\n2))')

$SH -c "$arith"
if test $? -ne 0; then
  echo 'failed'
fi

$SH -c "$arith_cr"
if test $? -ne 0; then
  echo 'failed'
fi

## STDOUT:
['3']
failed
## END

## OK mksh/ash/osh STDOUT:
['3']
['3']
## END

#### whitespace in string to integer conversion

tab=$(printf '\t42\t')
cr=$(printf '\r42\r')

$SH -c 'echo $(( $1 + 1 ))' dummy0 "$tab"
if test $? -ne 0; then
  echo 'failed'
fi

$SH -c 'echo $(( $1 + 1 ))' dummy0 "$cr"
if test $? -ne 0; then
  echo 'failed'
fi

## STDOUT:
43
failed
## END

## OK mksh/ash/osh STDOUT:
43
43
## END

#### \r at end of line is not special

# hm I wonder if Windows ports have rules for this?

cr=$(printf 'argv.sh -\r')

$SH -c "$cr"

## STDOUT:
['-\r']
## END

#### Default IFS does not include \r \v \f

# dash and zsh don't have echo -e
tab=$(printf -- '-\t-')
cr=$(printf -- '-\r-')
vert=$(printf -- '-\v-')
ff=$(printf -- '-\f-')

$SH -c 'argv.sh $1' dummy0 "$tab"
$SH -c 'argv.sh $1' dummy0 "$cr"
$SH -c 'argv.sh $1' dummy0 "$vert"
$SH -c 'argv.sh $1' dummy0 "$ff"

## STDOUT:
['-', '-']
['-\r-']
['-\x0b-']
['-\x0c-']
## END

# No word splitting in zsh

## OK zsh STDOUT:
['-\t-']
['-\r-']
['-\x0b-']
['-\x0c-']
## END

