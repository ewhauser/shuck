#!/bin/bash

name=abc
suffix=b

# Invalid: a variable expansion is used inside the pattern.
trimmed=${name%$suffix}

# Invalid: replacement patterns have the same problem.
rewritten=${name/$suffix/x}

# Valid: literal patterns are fine.
trimmed=${name%b}
rewritten=${name/ab/x}
