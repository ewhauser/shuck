#!/bin/sh

# Should trigger: direct declare command in sh
declare portable=value
printf '%s\n' "$portable"

# Should trigger: wrapped declare command still resolves to declare
command declare wrapped=value
printf '%s\n' "$wrapped"

# Should not trigger: plain portable assignment
plain=value
printf '%s\n' "$plain"
