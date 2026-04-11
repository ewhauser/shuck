#!/bin/bash
find ./ -name *.jar
find ./ -name "$prefix"*.jar
for f in $(find ./ -name *.cfg); do :; done
printf '%s\n' "$(find . -path */tmp/*)"

find ./ -name '*.jar'
find ./ -name \*.tmp
find ./ -path \*/tmp/\*
find ./ -type f*
command find ./ -name *.jar
find ./ -name "$pattern"
