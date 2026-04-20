#!/usr/bin/env bash
n=1
x=1
limit=3
declare -a ver
declare -a items
declare -A assoc
i=0
key=name
m=$(($n + 1))
(( $x + 1 ))
(( ${x} + 1 ))
for (( i=$limit; i > 0; i-- )); do :; done
items[$i]=x
items[$i+1]=y
items[${key},27]=z
assoc[$key]=x
(( ${ver[0]} + 1 ))
(( ${assoc[key]} + 1 ))
(( ${ver[$i]} + 1 ))
(( $1 + 1 ))
(( ${#x} + 1 ))
