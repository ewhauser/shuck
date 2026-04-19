#!/bin/sh

# Should trigger: bracket globs that are trying to spell out whole words or lists
foo[appname] arg
echo usage: cmd [start\|stop\|restart]
printf '%s\n' "$dir"/[appname]
ITEM=[0,-1,1,-10,-20]
cat <<EOF >/etc/systemd/system/[appname].service
EOF

# Should not trigger: valid single-character sets and literal bracket text
echo [ab]
echo [a-z]
echo [[:alpha:]]
echo foo[bar]baz
echo "usage: cmd [start\|stop\|restart]"
echo \[appname\]
