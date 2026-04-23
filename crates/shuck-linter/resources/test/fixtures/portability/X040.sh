#!/bin/sh
if [ $tools[kops] ]; then :; fi
if [ "${tools[kops]}" ]; then :; fi
if [ ${#tools[@]} -eq 0 ]; then :; fi
if [ ${cost%[\.]*} -lt 10 ]; then :; fi
if [ '$tools[kops]' ]; then :; fi
if [ \$tools[kops] ]; then :; fi
if [ "\$tools[kops]" ]; then :; fi
if [ "$tools" ]; then :; fi
