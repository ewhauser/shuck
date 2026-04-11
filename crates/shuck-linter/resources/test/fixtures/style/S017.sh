#!/bin/bash

x='a b'
arr=($x prefix$x $@ $* ${items[@]} ${x:-a b} $HOME/*.txt)
declare listed=($x)
arr+=($tail)

arr=("safe $x" "${items[@]}")
arr=([k]=$x)
arr=($(printf '%s\n' one two))
value=$x
