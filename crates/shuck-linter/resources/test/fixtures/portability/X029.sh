#!/bin/sh

# Should trigger: exact upper-case ranges in tr operands are locale-dependent
tr A-Z xyz < foo
tr abc A-Z < foo
tr A-Z a-z < foo
tr -d A-Z < foo
tr -s 'A-Z' < foo
tr -- "A-Z" xyz < foo

# Should not trigger: bracketed forms and lookalikes are different tr diagnostics
tr '[A-Z]' xyz < foo
tr AA-Z xyz < foo
tr A-ZA xyz < foo
command tr A-Z xyz < foo
builtin tr A-Z xyz < foo
