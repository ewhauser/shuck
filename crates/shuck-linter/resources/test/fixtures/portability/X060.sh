#!/bin/sh
[[ -v myvar ]] || :
[[ -n ${myvar+set} ]] || :
