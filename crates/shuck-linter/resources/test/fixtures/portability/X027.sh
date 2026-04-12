#!/bin/sh

# Should trigger: echo flag handling depends on the shell implementation
echo -n hi
echo -e hi
echo -nn hi
value=$(echo -ne "hello")
echo "-s" hi
echo '-e' hi

# Should not trigger: these are plain operands, non-portable-lookalike literals, or wrapped echo calls
echo -- hi
echo -x hi
echo -nfoo hi
echo '-I' hi
command echo -n hi
builtin echo -n hi
