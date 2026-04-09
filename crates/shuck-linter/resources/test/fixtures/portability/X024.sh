#!/bin/sh

# Should trigger: uppercase case-modification expansion
printf '%s\n' "${1^^}" "${1^}" "${name^^pattern}"

# Should trigger: lowercase case-modification expansion
printf '%s\n' "${name,,}" "${name,}" "${name,,pattern}"

# Should trigger: array-based case modification still uses the same bash-only form
printf '%s\n' "${arr[@]^^}" "${arr[0]^}"

# Should trigger: case modification in an expanding heredoc body
cat <<EOF
Expected: '${commit^^}'
EOF

# Should not trigger: other transformation operators are separate
printf '%s\n' "${name@Q}" "${!name^^}" "${name//x/y}"
