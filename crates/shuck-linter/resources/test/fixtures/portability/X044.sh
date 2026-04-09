#!/bin/sh

# Should trigger: nested target with an outer zsh operation.
x=${${(M)path:#/*}:-$PWD/$path}

# Should not trigger: plain modifier group belongs to X043.
y=${(f)foo}

# Should not trigger: nested target without an outer operation belongs to X051.
versions=(${${(f)"$(echo test)"}})
