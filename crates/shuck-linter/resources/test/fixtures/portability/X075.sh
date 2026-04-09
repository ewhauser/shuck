#!/bin/sh
if [ -O "$file" ]; then :; fi
if ! test -O "$file"; then :; fi
if [ ! -O "$file" -a -w "$file" ]; then :; fi
if [ -w "$file" ]; then :; fi
