#!/bin/sh

dir=.

# Should trigger: sourced file has an extra argument.
. "$dir"/setup.sh foo

# Should trigger: more than one extra argument still starts at the first extra word.
. ./helper.sh alpha beta

# Should not trigger: plain dot invocation.
. ./setup.sh
