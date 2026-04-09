#!/bin/sh

grep ^start'\s'end file.txt
printf '%s' 'foo\nbar'
printf '%s' 'foo\bar'
printf '%s' 'foo\x41bar'
printf '%s' 'foo\'
