#!/bin/sh
[ "$x" = (foo|bar)* ]
[ "$x" = @(foo) ]
[ "$x" = !(name) ]
[ "$x" = '(foo|bar)*' ]
[ "$x" = foo ]
[ "$x" = @(foo|bar) ]
[ "$x" = '@(foo|bar)' ]
