#!/bin/sh

# Should trigger: nested zsh expansion without an outer operation.
versions=(${${(f)"$(echo test)"}})

# Should not trigger: nested target with an outer operation belongs to X044.
x=${${(M)path:#/*}:-$PWD/$path}
