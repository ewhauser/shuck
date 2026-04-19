#!/bin/bash

set -- 'a b'
HOME=/home/me
dir='build dir'
file='artifact*'
name='a b'
options='-j 8'
unset debug

printf '%s\n' $1
cp $HOME/$dir/dist/bin/$file /tmp
echo "ok $(printf '%s\n' $name)"
make $options file

printf '%s\n' "$1"
cp "$HOME/$dir/dist/bin/$file" /tmp
echo "ok $(printf '%s\n' "$name")"
bash ${debug:+"-x"} script
