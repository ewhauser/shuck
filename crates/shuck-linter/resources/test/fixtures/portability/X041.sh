#!/bin/sh
if [[ $words[2] = */ ]]; then :; fi
if [[ -v assoc["$key"] ]]; then :; fi
if [[ '$words[2]' = */ ]]; then :; fi
if [[ \$words[2] = */ ]]; then :; fi
if [[ "\$words[2]" = */ ]]; then :; fi
if [[ "$words" = */ ]]; then :; fi
