#!/bin/bash

# Invalid: this treats a literal pipeline as a truthy string.
[ "lsmod | grep v4l2loopback" ]

# Invalid: unary string tests do not execute the quoted pipeline either.
[ -n "modprobe | grep snd" ]

# Invalid: nested `[[ ]]` conditions still only see a string literal here.
[[ "$ok" && "lsmod | grep v4l2loopback" ]]

# Valid: plain quoted strings are covered by the generic constant-test rules instead.
[ "echo hi" ]

# Valid: explicit comparisons are outside this quoted-command rule.
[ "cat file | grep foo" = x ]
