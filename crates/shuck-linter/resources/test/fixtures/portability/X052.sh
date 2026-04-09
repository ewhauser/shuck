#!/bin/sh

# Should trigger: function keyword with parens in sh
function with_parens() { :; }

# Should not trigger: function keyword without parens belongs to X004
function keyword_only { :; }
