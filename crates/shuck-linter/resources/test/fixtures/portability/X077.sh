#!/bin/sh

# Current oracle should not trigger on nested default operands.
printf '%s\n' "${outer:-${inner:-fallback}}"

# Should not trigger: the default arm is a plain literal.
printf '%s\n' "${outer:-fallback}"
