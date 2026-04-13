#!/bin/sh
# shellcheck disable=2046
if test "$(expr substr $(uname -s) 1 5)" = "Linux"; then echo linux; fi
