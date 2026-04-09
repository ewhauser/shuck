#!/bin/bash

cat input.txt > c99
cat input.txt >> grep
cat input.txt 2> sed
cat input.txt < awk
cat input.txt >| command

cat input.txt > "cat"
cat input.txt > ./cat
cat input.txt > /tmp/cat
cat input.txt > cat.txt
cat input.txt > "$name"
cat input.txt > ${name}
cat input.txt <<< cat
cat input.txt > true
