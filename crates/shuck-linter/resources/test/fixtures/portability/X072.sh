#!/bin/sh

unset -v "${!prefix_@}"
unset -v x${!prefix_*}
unset -f "${!func_@}"
unset -v "${!name}"
