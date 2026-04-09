#!/bin/sh
if [ -k "$file" ]; then :; fi
if ! test -k "$file"; then :; fi
if [ "$file" = "-k" ]; then :; fi
if [ -w "$file" ]; then :; fi
if [ ! -k "$file" -a -w "$file" ]; then :; fi
