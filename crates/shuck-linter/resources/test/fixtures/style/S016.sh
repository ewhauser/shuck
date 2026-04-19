#!/bin/sh

# Should trigger: direct echo inside a command substitution.
direct=$(echo direct)

# Should trigger: command and builtin wrappers still count.
wrapped=$(command echo wrapped)
builtin_wrapped=$(builtin echo builtin)
redirected=$(echo hidden >/dev/null)

# Should trigger: the inner substitution is the one that contains echo.
outer=$(foo $(echo nested))
quoted_outer=$(foo "$(echo quoted)")

# Should not trigger: a path to echo is not the builtin echo command.
path_plain=$(/bin/echo path)

# Should not trigger: a pipeline changes the shape enough to avoid this rule.
pipeline=$(echo piped | tr a-z A-Z)

# Should not trigger: substitutions without echo are fine.
printf_subst=$(printf '%s\n' value)

# Should not trigger: a single nested command substitution belongs to a different warning family.
nested_only=$(echo $(basename "$path" .txt))
quoted_nested_only=$(echo "$(basename "$path" .txt)")

# Should not trigger: echo can still matter when a dynamic prefix can collapse into a dash-led first argument.
dynamic_dash=$(echo ${prefix}-suffix)
quoted_dynamic_dash=$(echo "${prefix}-suffix")

# Should not trigger: echo feeding another command in a pipeline is not the final substitution body.
pipeline_cut=$(echo "$line" | cut -d' ' -f2-)

# Should not trigger: echo outside command substitution is covered elsewhere.
echo "$(date)"
