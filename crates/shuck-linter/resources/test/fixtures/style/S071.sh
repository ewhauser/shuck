#!/bin/bash
CFLAGS="${SLKCFLAGS}" ./configure --target="$ARCH-slackware-linux"
A="$a" B="${b:-fallback}" command run
C="$left""$right" command run
CFLAGS=$SLKCFLAGS ./configure
CFLAGS="~" ./configure
CFLAGS="prefix$SLKCFLAGS" ./configure
CFLAGS="${arr[@]}" ./configure
export CFLAGS="${SLKCFLAGS}"
CFLAGS="${SLKCFLAGS}"
