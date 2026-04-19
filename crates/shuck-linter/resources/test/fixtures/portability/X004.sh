#!/bin/sh

# Should trigger: function keyword without trailing parens in sh
function plain
{
  :
}

# Should not trigger: function keyword with trailing parens belongs to X052
function paren() { :; }

# Should not trigger: portable function definition
portable() { :; }
