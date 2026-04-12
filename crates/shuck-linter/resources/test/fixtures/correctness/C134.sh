#!/bin/sh

# Should trigger: getopts declares -o, but the matching case never handles it.
while getopts ':a:d:o:h' OPT; do
  case "$OPT" in
    a) alg=$OPTARG ;;
    d) domain=$OPTARG ;;
    h) echo help; exit 0 ;;
  esac
done

# Should not trigger: one arm can handle multiple options.
while getopts ':ab' opt; do
  case "$opt" in
    a|b) : ;;
  esac
done

# Should not trigger: only cases over the getopts variable are correlated.
while getopts ':xy' opt; do
  case "$other" in
    x|y) : ;;
  esac
done
