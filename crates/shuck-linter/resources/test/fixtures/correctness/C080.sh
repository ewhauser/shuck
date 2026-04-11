#!/bin/sh
grep start* out.txt
grep "start*" out.txt
grep 'foo*bar' out.txt
grep foo*bar out.txt
grep -efoo* out.txt
grep --regexp start* out.txt
grep --exclude '*.txt' foo*bar out.txt
grep --label stdin foo*bar out.txt
grep "foo*bar" out.txt
grep item\* out.txt
grep -E "foo*bar" out.txt

grep "a.*" out.txt
grep a.* out.txt
grep "[ab]*" out.txt
grep [ab]* out.txt
grep '*start' out.txt
grep '*start*' out.txt
grep item\\* out.txt
grep '^ *#' out.txt
grep '"name": *"$x"' out.txt
grep -F foo*bar out.txt
grep -F "foo*bar" out.txt
grep --fixed-strings foo*bar out.txt
grep --fixed-strings "foo*bar" out.txt
grep -eo foo* out.txt
grep --regexp='start*' out.txt
grep --regexp=start* out.txt
grep -efoo out.txt
