#!/bin/sh

printf '%s\n' done
printf '%s\n' do
command done
x=done printf '%s\n' ok
export x=done
rvm 3.1.3 do rvm gemdir
: do
[ "$state" = done ]
[[ $state == done ]]
[[ $state == do ]]
case done in ok) :;; esac
for value in done; do :; done
echo hi > done
trap done EXIT

echo "done" 'done' d"on"e
echo "do" 'do' d"o"
echo done.x done#suffix
./do
case "$state" in done) :;; esac
[[ $state =~ done ]]
echo ${state:-done}
