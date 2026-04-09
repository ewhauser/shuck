#!/bin/sh

# Should trigger: zsh-only operator in a POSIX shell script.
echo "first" &|
: &|

# Should not trigger: standard backgrounding.
echo "second" &

# Should not trigger: regular pipeline.
echo "third" | cat

# Should not trigger: literal text.
echo "&|"
