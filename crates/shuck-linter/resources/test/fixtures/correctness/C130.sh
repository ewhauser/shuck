#!/bin/sh
# shellcheck disable=2154,3024,2086,2034,2090
myfunc() {
	CFLAGS+=" -DDIR=\"$PREFIX/share/\""
	$CC $CFLAGS -c test.c -o test.o
}
myfunc
