#!/bin/bash

re='a+'

# Invalid: quoting the regex makes bash treat it as a literal string
[[ $value =~ "$re" ]]

# Invalid: quoted literal regexes are also treated literally
[[ foo =~ "a+" ]]

# Valid: unquoted regexes keep regex semantics
[[ $value =~ $re ]]
