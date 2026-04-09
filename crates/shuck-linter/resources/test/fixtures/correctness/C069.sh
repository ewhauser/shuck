#!/bin/bash
# shellcheck disable=2006
ARCH=`uname -a | cut -f12 -d\ `
echo "$ARCH"
