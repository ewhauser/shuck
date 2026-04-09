#!/bin/sh
if [[ $term == @(xterm|screen)* ]]; then :; fi
if [[ $term == *.sh ]]; then :; fi
if [[ $term != @(foo|bar) ]]; then :; fi
