#!/bin/sh

# Should trigger: echo backslash escapes depend on the shell
echo \n
echo "\\n"
echo '\n'
echo foo\nbar
echo "foo\nbar"
echo 'foo\nbar'
echo \x41
echo \077

# Should not trigger: these are different cases
echo \c
echo \u1234
command echo \n
builtin echo \n
printf '%s\n' \n
