#!/bin/sh

# Should trigger: string append assignment
x+="hello"

# Should trigger: declaration append assignment
readonly value+="suffix"

# Should trigger: subscript append assignment
index[1+2]+="world"

# Should not trigger: arithmetic update operators
(( i += 1 ))
