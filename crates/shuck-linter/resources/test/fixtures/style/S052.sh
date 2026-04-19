#!/bin/sh
[ -n $foo ]
[ -n ${bar} ]
[ -n prefix$baz ]
[ -n ${qux:-fallback} ]

[ -n "$foo" ]
test -n $foo
test -z $foo
[ -n literal ]
test -n $(printf '%s\n' "$foo")
test -n ${qux:-fallback}
[ -n ${arr[*]} ]
