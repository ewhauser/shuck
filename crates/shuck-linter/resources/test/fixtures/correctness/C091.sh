#!/bin/bash

# Invalid: the quoted home-relative path stays literal in `[ ]`.
[ "$profile" = "~/.bashrc" ]

# Invalid: either side of the string comparison can carry the quoted `~/...`.
[ "~/.bash_profile" = "$profile" ]

# Invalid: `[[ ]]` string comparisons have the same quoted-tilde issue.
[[ "$profile" == "~/.zshrc" ]]

# Invalid: single quotes still prevent tilde expansion.
[ "$profile" != '~/.config/fish/config.fish' ]

# Invalid: assignments and command arguments have the same quoted-tilde issue.
profile='~/.bash_profile'
printf '%s\n' "~/.config/powershell/profile.ps1"
[ -e "~/.cache/app" ]

# Valid: an unquoted tilde expands before the comparison.
[ "$profile" = ~/.bashrc ]

# Valid: `~user` is a different lookup and not interchangeable with `$HOME`.
[ "$profile" = "~user/.bashrc" ]
