#!/bin/sh

# Invalid: backtick output becomes the command name.
`echo hello` | cat
if `echo true`; then :; fi
FOO=1 `echo run`

# Valid: wrappers, quoting, and argument positions are outside this rule.
command `echo hello`
"`echo hello`" | cat
x`echo hello`
echo `date`
