#!/bin/sh
<<'DOC'
This block is never read by a command.
DOC

cat <<'DOC'
This block is consumed by cat.
DOC
