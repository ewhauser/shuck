#!/bin/sh
grep start* out.txt
grep -e item? out.txt
grep --regexp item,[0-4] out.txt
grep -Eq item,[0-4] out.txt
grep -eo item* out.txt
grep -F -- item,[0-4] out.txt

grep "start*" out.txt
grep --regexp='item,[0-4]' out.txt
grep --regexp=item,[0-4] out.txt
grep -eitem* out.txt
grep -f patterns.txt item,[0-4] out.txt
