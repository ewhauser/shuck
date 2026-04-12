#!/bin/bash
cat < foo > foo
sort foo > foo
unzip -p "$1" test.c > test.c
cat < "$src" > "$src"
echo "$(cat "$f")" | sed 's/x/y/' >"$f"
printf '%s\0' **/* | bsdtar --null --files-from - --exclude .MTREE | gzip -c -f -n > .MTREE
{ [[ "$f" == /dev/null ]] || set -x; } &>"$f"
exec 4<> "$LOG_PATH"
cat < foo > bar
sort foo > bar
cat < "$src" > "$dst"
