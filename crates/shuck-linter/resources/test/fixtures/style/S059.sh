#!/bin/sh
tempfile -n "$TMPDIR/Xauthority"
tempfile
command tempfile -n "$TMPDIR/Xauthority"
sudo tempfile -n "$TMPDIR/Xauthority"
alias tempfile=mktemp
