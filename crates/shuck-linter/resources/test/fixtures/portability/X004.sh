#!/bin/sh

# Should trigger: function keyword without trailing parens in sh
function plain { :; }

# Should trigger: function keyword with trailing parens is also non-portable in sh
function paren() { :; }

# Should not trigger: portable function definition
portable() { :; }
