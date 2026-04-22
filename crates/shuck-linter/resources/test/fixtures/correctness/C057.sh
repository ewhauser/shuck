#!/bin/sh

# Compatibility note: the pinned oracle currently stays quiet on this form.
out=$(printf hi > out.txt)

# Compatibility note: the pinned oracle also stays quiet on this reroute.
out=$(printf hi >&2)

# Baseline direct capture.
out=$(printf hi)
