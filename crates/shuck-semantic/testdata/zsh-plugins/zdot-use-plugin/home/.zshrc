#!/bin/zsh

# zdot_use_plugin accepts repository specs and resolves this one as the
# standalone zsh-autosuggestions plugin root.
ZSH_AUTOSUGGEST_STRATEGY=(match_prev_cmd completion)
zdot_use_plugin zsh-users/zsh-autosuggestions defer

