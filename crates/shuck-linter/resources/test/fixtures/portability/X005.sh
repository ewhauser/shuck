#!/bin/sh

case "$mode" in
  start)
    printf '%s\n' "starting"
    ;&
  stop)
    printf '%s\n' "stopping"
    ;;&
  *)
    printf '%s\n' "default"
    ;;
esac
