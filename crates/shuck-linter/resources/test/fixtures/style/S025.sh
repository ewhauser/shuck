#!/bin/sh

echo \q
printf '%s\n' \q
echo \command
echo foo\xbar
foo=bar\w
case x in foo\q) : ;; esac
cat < foo\q
echo \n
echo \Q
echo "\q"
echo '\q'
