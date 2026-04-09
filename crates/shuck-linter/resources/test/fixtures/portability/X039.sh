#!/bin/sh

# Should trigger: zsh always block.
{ :; } always { :; }

# Should trigger: multiline always block.
{
  :
} always {
  :
}

# Should not trigger: ordinary brace group.
{
  :
}
