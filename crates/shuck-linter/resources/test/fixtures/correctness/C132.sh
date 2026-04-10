#!/bin/bash

# Should trigger: typo in configure option name.
./configure --with-optmizer="${CFLAGS}"

# Should trigger: typo in quoted configure option.
configure "--enable-optmizer=${CFLAGS}"

# Should not trigger: correctly spelled option name.
./configure --with-optimizer="${CFLAGS}"

# Should not trigger: non-configure command using a similar argument.
make --with-optmizer="${CFLAGS}"
