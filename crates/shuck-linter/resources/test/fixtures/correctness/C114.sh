#!/bin/sh
for i in $CWD/file.*pattern*; do :; done
for i in ${CWD}/file.*pattern*; do :; done
for i in $(pwd)/file.*pattern*; do :; done

for i in "$CWD"/file.*pattern*; do :; done
for i in file.*pattern*; do :; done
for i in "$CWD"/*.txt; do :; done
for i in $DIR/setjmp-aarch64/{setjmp.S,private-*.h}; do :; done
