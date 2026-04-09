#!/bin/sh

# Should trigger: indexed array element expansion
printf '%s\n' "${arr[0]}"

# Should trigger: array splats are still array references
printf '%s\n' "${arr[@]}" "${arr[*]}"

# Should not trigger: these forms belong to more specific portability checks
printf '%s\n' "${#arr[0]}" "${#arr[@]}" "${arr[0]%x}" "${arr[0]:2}" "${arr[0]//x/y}" "${arr[0]:-fallback}" "${!arr[0]}" "${!arr[@]}"
