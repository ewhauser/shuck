#!/bin/bash
if [ 1 + 2 -eq 3 ]; then :; fi
if test 1 + 2 -eq 3; then :; fi
if [[ 1 + 2 -eq 3 ]]; then :; fi
if [ "$x" + 1 -eq 2 ]; then :; fi
if [ $((1 + 2)) -eq 3 ]; then :; fi
if [[ $((1 + 2)) -eq 3 ]]; then :; fi
