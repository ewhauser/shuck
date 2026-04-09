#!/bin/sh
[ "$x" = (foo|bar)* ]
[ "$x" = '(foo|bar)*' ]
[ "$x" = foo ]
[ "$x" = @(foo|bar) ]
[ "$x" = '@(foo|bar)' ]
