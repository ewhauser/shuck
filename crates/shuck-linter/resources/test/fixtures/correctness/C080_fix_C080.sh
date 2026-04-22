#!/bin/sh
grep start* out.txt
grep 'foo\*bar*' out.txt
grep item\* out.txt
grep --regexp='start*' out.txt
grep --fixed-strings foo*bar out.txt
