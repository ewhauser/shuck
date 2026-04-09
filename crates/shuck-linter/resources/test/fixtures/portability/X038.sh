#!/bin/sh

# Should trigger: compact zsh-style brace body.
if [[ -n "$x" ]] { :; }

# Should trigger: multiline zsh-style brace if.
if [[ -n "$x" ]] {
  :
} elif [[ -n "$y" ]] { :; } else { :; }

# Should not trigger: standard if syntax.
if true; then
  :
fi
