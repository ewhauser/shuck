#!/bin/zsh

add-zsh-hook precmd _zsh_autosuggest_start

_zsh_autosuggest_start() {
  _zsh_autosuggest_fetch
}

_zsh_autosuggest_fetch() {
  strategies=(${=ZSH_AUTOSUGGEST_STRATEGY})
}

