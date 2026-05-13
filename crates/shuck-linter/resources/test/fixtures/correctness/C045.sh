#!/bin/sh

--- a/sample.txt
  --- indented.txt
--help
\-n foo
-$tool foo

echo --- a/sample.txt
# --- comment.txt
cat <<'DOC'
--- heredoc.txt
DOC
"-n" foo
'-n' foo
command -n foo
find . -exec chmod 755 {}\; -o -name '*.txt'
