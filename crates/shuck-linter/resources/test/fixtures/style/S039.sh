#!/bin/sh

grep ^start'\s'end file.txt
printf '%s\n' '\n'foo
printf '%s\n' 'ab\n'c
printf '%s\n' '\\n'foo
printf '%s\n' 'foo\nbar'
printf '%s\n' '\x'41
printf '%s\n' '\0'foo
printf '%s\n' $'\n'foo
printf '%s\n' a'\'bc
