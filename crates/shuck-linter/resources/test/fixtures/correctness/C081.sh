#!/bin/bash
if [[ $mirror == $pkgs ]]; then echo same; fi
if [[ "$a" = $1 ]]; then :; fi
if [[ "$a" != ${b%%x} ]]; then :; fi
if [[ "$a" == ${arr[0]} ]]; then :; fi
printf '%s\n' "$( [[ $mirror == $pkgs ]] && echo same )"

if [[ "$a" == "$b" ]]; then :; fi
if [[ "$a" == $b* ]]; then :; fi
if [[ "$a" == $b$c ]]; then :; fi
if [[ "$a" == ${b}_x ]]; then :; fi
if [[ "$a" < $b ]]; then :; fi
if [ "$a" = $b ]; then :; fi
