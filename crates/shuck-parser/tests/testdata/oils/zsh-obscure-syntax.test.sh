## compare_shells: zsh

#### anonymous function with scoped options and argv

() {
  emulate -L zsh
  setopt extendedglob
  local -a matches=(src/**/*.zsh(.N:t:r))
  print -r -- ${(j:,:)matches}
} one two

#### brace control flow with always cleanup

{
  if [[ -n ${commands[zsh]} ]] {
    print -r -- ok
  } elif (( ${+commands[false]} )) {
    print -r -- maybe
  } else {
    print -r -- missing
  }
} always {
  print -r -- cleanup
}

#### repeat and foreach short loop forms

repeat 2 print -r -- tick

foreach item (alpha beta gamma) {
  print -r -- ${item:u}
}

for key value in ${(kv)parameters}; {
  [[ $key == path ]] && print -r -- $value
}

#### typed associative array assignment and subscript flags

typeset -A colors=(
  [normal]=black
  [warning]=yellow
  [error]=red
)
print -r -- ${colors[(i)warn*]} ${colors[(r)r*]}
colors[(I)e*]=brightred

#### nested parameter substitutions with colon modifiers

local target=/tmp/archive.tar.gz
print -r -- ${${target:t}:r} ${${:-$target}:h}
print -r -- ${(Q)${:-one\ two}} ${(%)${:-%n@%m}}

#### parameter flags with delimiter arguments

local -a words=(one two three)
print -r -- ${(j:|:)words}
print -r -- ${(ps:\0:)${:-one${(l:1::\0:):-}two}}
print -r -- ${(qqq)${(F)words}}

#### glob qualifiers and directory stack modifiers

print -r -- **/*.zsh(.DN:t:r)
print -r -- *(^@N) *(.om[1,3])

#### equals and command process substitution words

diff =(print -r -- left) <(print -r -- right)
cat > >(sed 's/^/[out] /') <<< ${:-payload}

#### zsh multios and pipe redirection

print -r -- message >out.log >audit.log
print -r -- quiet &>/dev/null &|

#### case pattern operators and fallthrough terminators

case $OSTYPE in
  (darwin|freebsd)<->)
    print -r -- bsd ;|
  linux(|-gnu))
    print -r -- linux ;&
  *)
    print -r -- other ;;
esac

#### extended glob patterns in conditionals

if [[ $file == (#b)(*/)([^/]##).(zsh|plugin)(#e) ]]; then
  print -r -- $match[1] $match[2]
fi

if [[ $name == (#i)readme.(md|txt) && $path == (#s)*/docs/*(#e) ]]; then
  print -r -- docs
fi

#### zparseopts style option specs and command modifiers

zparseopts -D -E -F -- \
  h=help -help=help \
  v+:=verbose -verbose+:=verbose \
  o:=output -output:=output

noglob command print -r -- **/*(N)
whence -m 'z*' >/dev/null

#### prompt escapes and ${(%)...} inside assignment

PS1=$'%F{green}%n%f:%~ %# '
local rendered=${(%)PS1}
print -r -- $rendered

#### arithmetic with zsh subscript expressions

integer count=${#path}
(( count += ${+commands[git]} ? path[(I)*bin*] : 0 ))
print -r -- $count

#### coproc block with zsh-style body

coproc {
  print -r -- request
  read -r reply
  print -r -- $reply
}
