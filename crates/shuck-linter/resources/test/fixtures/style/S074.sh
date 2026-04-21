#!/bin/bash
printf '%s\n' x &;
printf '%s\n' y & ;

printf '%s\n' ok &
wait

case ${1-} in
  break) printf '%s\n' ok &;;
  spaced) printf '%s\n' ok & ;;
  fallthrough) printf '%s\n' ok & ;&
  continue) printf '%s\n' ok & ;;&
esac
