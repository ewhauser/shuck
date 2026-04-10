#!/bin/bash

# Should trigger: repeated spacing between echoed arguments collapses.
echo foo    bar
echo -n    "foo"
echo "foo"    bar
echo foo    "bar"

# Should not trigger: single gaps and single-argument layout are fine.
echo foo  bar
echo    foo

# Should not trigger: wrapped echoes are intentionally skipped.
command echo foo    bar
builtin echo foo    bar
