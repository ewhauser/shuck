#!/bin/sh
# shellcheck disable=2086,2154
find $dir -type f -name "rename*" -execdir sh -c 'mv {} $(echo {} | sed "s|rename|perl-rename|")' \;
