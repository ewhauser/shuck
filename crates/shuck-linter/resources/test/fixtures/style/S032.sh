#!/bin/sh

false || remove=set

true && remove=set

remove=set || echo nope

true && remove=set && echo later

[ -n "$x" ] && domain=$domain || domain=$str

echo ok && remove=set

foo=bar && baz=qux
