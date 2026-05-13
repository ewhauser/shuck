#!/bin/bash

[! -r /etc/passwd ]
[foo]
[foo ]
[[foo]]
[[foo ]]
if [! -r x ]; then :; fi
case x in a)[! -r x ];; esac

[ ! -r /etc/passwd ]
[ foo]
[[ foo]]
if[ -r x ]; then :; fi
echo [foo]
