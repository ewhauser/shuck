#!/bin/sh

# Should trigger: -k is handled in the case statement but missing from getopts.
while getopts ':a:d:h' OPT; do
  case "$OPT" in
    a) alg=$OPTARG ;;
    d) domain=$OPTARG ;;
    k) keyfile=$OPTARG ;;
    h) echo help; exit 0 ;;
  esac
done

# Should trigger only on the undeclared alternative.
while getopts 'a' opt; do
  case "$opt" in
    a|b) : ;;
  esac
done

# Should not trigger: getopts error handlers and fallback arms are intentional.
while getopts ':a' opt; do
  case "$opt" in
    a) : ;;
    \?) : ;;
    :) : ;;
    *) : ;;
  esac
done
