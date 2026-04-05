#!/bin/sh

# Invalid: loop control at top level has nothing to control
break
continue 2

while true; do
  # Valid: inside a loop, break is meaningful
  break
done

for item in a; do
  # Valid: continue is fine here
  continue
done
