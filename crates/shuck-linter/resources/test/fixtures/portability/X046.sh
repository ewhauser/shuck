#!/bin/sh
[ "$x" = (foo|bar)* ]
[ "$x" = foo ]
[ "$x" = @(foo|bar) ]
