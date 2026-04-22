#!/bin/sh

# Should trigger: stderr is duplicated to stdout before stdout is redirected
echo ok 2>&1 >/dev/null
echo ok 2>&1 >>file
echo ok 2>&1 1>/dev/null
echo ok 2>&1 3>aux >out

# Should not trigger: stdout is redirected first
echo ok >file 2>&1

# Should not trigger: stderr goes to a separate file before stdout is redirected
echo ok 2>err >out

# Should not trigger: later descriptor duplication does not redirect stdout to a file
echo ok 2>&1 1>&3
