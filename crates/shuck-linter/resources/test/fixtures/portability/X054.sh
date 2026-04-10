#!/bin/sh
echo @(foo|bar)
case "$x" in
  @(foo|bar)) : ;;
esac
trimmed=${name%@($suffix|$(printf '%s' zz))}
