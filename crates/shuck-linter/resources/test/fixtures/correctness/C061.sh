#!/bin/bash
# shellcheck disable=2154
"$root/pkg/{{name}}/bin/{{cmd}}" "$@"

echo "{{name}}"
command "{{tool}}"
printf '%s\n' "$root/{{name}}/bin/{{cmd}}"
echo hi > "{{name}}"
"$root/bin/{{"
"$root/bin/}}"
