#!/bin/sh

# Should trigger: direct ls to grep pipeline.
ls | grep foo

# Should trigger: ls arguments still parse listing text output.
ls -1A /tmp | grep foo

# Should not trigger: wrapped utilities are intentionally ignored.
command ls | grep foo
ls | command grep foo

# Should not trigger: deprecated aliases are separate rules.
ls | egrep foo
ls | fgrep foo

# Should not trigger: non-grep consumer.
ls | awk '/foo/'
