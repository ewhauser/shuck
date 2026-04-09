#!/bin/sh

sed -i '$a\' "$1"
printf '%s\n' 'foo\bar'
