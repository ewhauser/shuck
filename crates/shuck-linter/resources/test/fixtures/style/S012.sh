#!/bin/sh

# Should trigger: direct ps to grep pipeline.
ps aux | grep foo

# Should trigger: grep variant flags still parse ps output text.
ps aux | grep -v grep

# Should not trigger: wrapped utilities are intentionally ignored.
command ps aux | grep foo
ps aux | command grep foo

# Should not trigger: non-grep consumer.
ps aux | awk '/foo/'

# Should not trigger: pid-targeted ps queries.
ps -p 1 -o comm= | grep -q systemd
ps -o command= -p "$parent" | grep -F -- "-f"

# Should not trigger: deprecated aliases are separate rules.
ps aux | egrep foo
ps aux | fgrep foo
