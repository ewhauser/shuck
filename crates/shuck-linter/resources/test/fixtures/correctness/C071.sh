#!/bin/sh
# shellcheck disable=2154,2009
((ps aux | grep foo) || kill "$pid") 2>/dev/null
