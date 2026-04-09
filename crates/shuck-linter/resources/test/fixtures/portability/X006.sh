#!/bin/sh

# Should trigger: input process substitution in an argument
cat <(printf '%s\n' hi)

# Should trigger: output process substitution as a redirect target
printf '%s\n' hi > >(wc -l)

# Should not trigger: plain command substitution is a different rule
printf '%s\n' "$(printf ok)"
