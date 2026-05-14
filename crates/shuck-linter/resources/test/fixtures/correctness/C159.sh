#!/bin/bash

count=0
count=1

mode=${mode:-dev}
mode=prod
mode=${mode#d}${mode:-prod}

refresh() {
  count=2
  local mode
  mode=test
}

readonly LIMIT=5
echo "$count" "$mode" "$LIMIT"
