#!/bin/sh

# Basic unused assignment
unused=1

# Used assignment (no diagnostic)
used=1
echo "$used"

# Exported variable (no diagnostic)
export EXPORTED=1
