#!/bin/bash
printf '%s\n' 'foo'Default'baz'
sed -i 's/${title}/'Default'/g' "$file"
x='a'b'c'
arr=('a'123'c')
printf '%s\n' 'foo'-'baz'
printf '%s\n' 'foo''baz'
printf '%s\n' 'foo'$bar'baz'
printf '%s\n' $'foo'Default'baz'
