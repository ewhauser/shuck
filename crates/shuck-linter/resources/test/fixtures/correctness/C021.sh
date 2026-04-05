#!/bin/sh

# Invalid: the case subject never changes
case x in
  x) : ;;
esac

# Valid: switching on runtime data is meaningful
case "$value" in
  x) : ;;
esac
