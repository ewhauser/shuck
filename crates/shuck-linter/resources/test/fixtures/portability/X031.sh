#!/bin/sh

# Should trigger: direct source builtin in sh
source ./helpers.sh

# Should trigger: wrapped source builtin still resolves to source
command source ./wrapped.sh

# Should trigger: ShellCheck reports the generic source portability code here too
inside_function() {
  source ./inside.sh
}

# Should not trigger: portable dot command
. ./portable.sh
