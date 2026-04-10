#!/bin/sh

# Should trigger: ANSI-C quoted assignment value
greeting=$'hello\n'

# Should trigger: ANSI-C quoted command argument
printf '%s\n' $'tab\t'

# Should trigger: ANSI-C quoted replacement operand
printf '%s\n' "${value//$'\n'/' '}"

# Should not trigger: ordinary single quotes stay portable
printf '%s\n' 'plain text'

# Should not trigger: trailing dollar before a closing single quote is literal
pattern="grep -q '^${name}$'"
