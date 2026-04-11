#!/bin/sh
[ -n $foo ]
test -n ${bar}
[ -n prefix$baz ]
test -n ${qux:-fallback}

[ -n "$foo" ]
test -z $foo
[ -n literal ]
test -n $(printf '%s\n' "$foo")
[ -n ${arr[*]} ]
