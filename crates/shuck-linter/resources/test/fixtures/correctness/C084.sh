#!/bin/sh
grep start* out.txt
grep -e item? out.txt
grep -eitem* out.txt
grep -oe item* out.txt
grep --regexp item,[0-4] out.txt
grep -Eq item,[0-4] out.txt
grep --exclude '*.txt' foo*bar out.txt
grep --label stdin item? out.txt
grep -F -- item,[0-4] out.txt
grep -F foo*bar out.txt
grep [0-9a-f]{40} out.txt
checksum="$(grep -Ehrow [0-9a-f]{40} ${template}|sort|uniq|tr '\n' ' ')"

grep "start*" out.txt
grep --regexp='item,[0-4]' out.txt
grep --regexp=item,[0-4] out.txt
grep -eo item* out.txt
grep -f patterns.txt item,[0-4] out.txt
grep --exclude '*.txt' "foo*bar" out.txt
grep \[ab\]\* out.txt
grep -F "foo*bar" out.txt
