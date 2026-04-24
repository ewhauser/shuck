#!/bin/sh
f(x) { :; }
function g(y) { :; }
coproc pycoproc (python3 "$pywrapper")
