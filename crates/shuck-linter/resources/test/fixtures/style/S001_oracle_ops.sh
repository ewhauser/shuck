#!/bin/bash

foo='a b/c'
bar='a b c'

printf '%s\n' ${foo#*/}
printf '%s\n' ${bar// /_}
printf '%s\n' ${bar^^}

printf '%s\n' "${foo#*/}" "${bar// /_}" "${bar^^}"
