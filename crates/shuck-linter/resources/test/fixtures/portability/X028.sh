#!/bin/sh

# Should trigger: exact lower-case ranges in tr operands are locale-dependent
tr a-z xyz < foo
tr abc a-z < foo
tr a-z A-Z < foo
tr -d a-z < foo
tr -s 'a-z' < foo
tr -- "a-z" xyz < foo

# Should not trigger: bracketed forms and lookalikes are different tr diagnostics
tr '[a-z]' xyz < foo
tr aa-z xyz < foo
tr a-zA xyz < foo
command tr a-z xyz < foo
builtin tr a-z xyz < foo
