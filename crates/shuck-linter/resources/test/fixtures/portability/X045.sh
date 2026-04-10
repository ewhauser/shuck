#!/bin/sh

# Should trigger: scalar append assignment
x=1
x+=64

# Should trigger: array-style append assignment
arr+=(one two)

# Should trigger: declaration assignment append
readonly value+=suffix

# Should trigger: subscript append assignment
index[1+2]+=3

# Should not trigger: arithmetic update operators
(( i += 1 ))
