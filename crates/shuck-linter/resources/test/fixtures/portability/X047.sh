#!/bin/sh

# Should trigger: the loop header binds two variables.
for key val in a 1 b 2; do
  echo "$key=$val"
done

# Should not trigger: standard single-target loop.
for entry in a b; do
  echo "$entry"
done
