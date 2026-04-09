#!/bin/sh

x="${${FALLBACK:-default}:-value}"

y="${fallback:-${value:-default}}"
echo '${${ignored}:-value}'
