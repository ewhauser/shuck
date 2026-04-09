#!/bin/sh

# Should trigger: zsh modifier group on a direct target.
x=${(f)foo}

# Should not trigger: nested target form belongs to later portability rules.
y=${${(M)path:#/*}:-$PWD/$path}

# Should not trigger: empty-target prompt expansion belongs to a different zsh form.
name=${(%):-%x}
