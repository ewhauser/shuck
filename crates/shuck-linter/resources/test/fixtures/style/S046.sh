#!/bin/bash
# shellcheck disable=2035
ls *.txt | xargs -n1 wc
