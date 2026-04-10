#!/bin/sh

# Should trigger: plain replacement expansion
printf '%s\n' "${1//a/b}" "${1/a/b}" "${name/#old/new}" "${name/%old/new}"

# Should trigger: array-based replacement still uses the same bash-only form
printf '%s\n' "${arr[0]//old/new}" "${arr[@]/old/new}" "${arr[*]//old}"

# Should trigger: replacement expansion in an expanding heredoc body
cat <<EOF
Expected: '${commit//old/new}'
EOF

# Should not trigger: neighboring parameter-expansion families are handled elsewhere
printf '%s\n' "${name^^}" "${name:1}" "${!name//old/new}" "${name@Q}"
