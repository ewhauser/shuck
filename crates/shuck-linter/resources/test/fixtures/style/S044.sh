#!/bin/bash

# Should trigger: an unquoted expansion flows from echo into sed's substitution.
echo $CLASSPATH | sed 's|foo|bar|g'
echo $HOME | sed -e 's|foo|bar|g'

# Should not trigger: quoted expansions and non-substitution sed usage.
echo "$KEEP" | sed 's|foo|bar|g'
echo $PATH | sed -n
