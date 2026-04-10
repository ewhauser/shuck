#!/bin/sh
case "$x" in
  foo_(a|b)_*) echo match ;;
esac
