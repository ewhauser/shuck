#!/bin/bash

HOME=/home/me
dir='build dir'
file='artifact*'
foo='a b'
name='a b'
cmd='printf %s\n hi'

printf '%s\n' prefix$foo
printf '%s\n' $foo-suffix
cp $HOME/$dir/dist/bin/$file /tmp
echo "ok $(printf '%s\n' $name)"
cat <<< $foo
printf '%s\n' ok >$file
$dir/tool --help

$cmd
for item in $foo; do
  printf '%s\n' "$item"
done
cp "$HOME/$dir/dist/bin/$file" /tmp
echo "ok $(printf '%s\n' "$name")"
cat <<< "$foo"
printf '%s\n' ok >"$file"
