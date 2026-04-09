#!/bin/sh
if true; then
  :
elif [[ "$OSTYPE" == foo ]]; then
  :
fi

if [[ "$OSTYPE" == foo ]]; then
  :
fi

if true; then
  :
elif [ "$OSTYPE" = foo ]; then
  :
fi
