#!/usr/bin/env bash
n=1
x=1
limit=3
m=$(($n + 1))
(( $x + 1 ))
(( ${x} + 1 ))
for (( i=$limit; i > 0; i-- )); do :; done
(( $1 + 1 ))
(( ${#x} + 1 ))
