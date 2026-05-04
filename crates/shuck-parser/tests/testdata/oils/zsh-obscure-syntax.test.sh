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

#### tree-sitter-zsh example getopts with nested case dispatch

while getopts ":wtfvh-:" opt; do
  case "$opt" in
    -)
      case "${OPTARG}" in
        wait)
          WAIT=1
          ;;
        help|version)
          REDIRECT_STDERR=1
          EXPECT_OUTPUT=1
          ;;
        foreground|benchmark|benchmark-test|test)
          EXPECT_OUTPUT=1
          ;;
      esac
      ;;
    w)
      WAIT=1
      ;;
    h|v)
      REDIRECT_STDERR=1
      EXPECT_OUTPUT=1
      ;;
    f|t)
      EXPECT_OUTPUT=1
      ;;
  esac
done

#### tree-sitter-zsh example array accumulation in pipeline loops

filelist=()
fid=0

find "$prefix/$folder" -type l | while read file; do
  target=`$readlink $file | grep '/\.npm/'`
  if [ "x$target" != "x" ]; then
    filelist[$fid]="$file"
    let 'fid++'
    base=`basename "$file"`
    find "`dirname $file`" -type l -name "$base"'*' \
      | while read link; do
          filelist[$fid]="$link"
          let 'fid++'
        done
  fi
done

if [ "${#filelist[@]}" -gt 0 ]; then
  for item in "${filelist[@]}"; do
    print -r -- "$item"
  done
fi

#### tree-sitter-zsh example mode dispatch with process substitution

args=()
if [[ -n $verbose ]]; then
  args+=("--reporter=spec")
else
  args+=("--reporter=singleline")
fi

case ${mode} in
  valgrind)
    valgrind                                     \
      --suppressions=./script/util/valgrind.supp \
      --dsymutil=yes                             \
      --leak-check=${leak_check}                 \
      $cmd "${args[@]}" 2>&1 |                   \
      grep --color -E '\w+_tests?.cc:\d+|$'
    ;;

  SVG)
    $cmd "${args[@]}" 2> >(grep -v 'Assertion failed' | dot -Tsvg >> index.html)
    ;;
esac

#### tree-sitter-zsh example filter function inside destination case

html_replace_tokens () {
  local url=$1
  sed "s|@NAME@|$name|g" \
    | sed "s|@URL@|$url|g" \
    | perl -p -e 's/<h1([^>]*)>(.*?)<\/h1>/<h1>\2<\/h1>/g' \
    | (if [ $(basename $(dirname $dest)) == "doc" ]; then
        perl -p -e 's/ href="\.\.\// href="/g'
      else
        cat
      fi)
}

case $dest in
  *.[1357])
    marked-man --roff $src | man_replace_tokens > $dest
    exit $?
    ;;
  *.html)
    (cat html/dochead.html && cat $src | marked && cat html/docfoot.html) \
      | html_replace_tokens $url > $dest
    ;;
esac

#### tree-sitter-zsh clean-core special parameter assignment

0=${(%):-%N}
