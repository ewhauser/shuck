#!/bin/sh

git branch -d `git branch --merged | grep -v '^*' | grep -v master | tr -d '\n'`
printf '%s\n' prefix`uname`suffix `date`

printf '%s\n' "`uname`" "$(date)" $(date)
stamp=`date`
arr=(`printf '%s\n' one two`)
