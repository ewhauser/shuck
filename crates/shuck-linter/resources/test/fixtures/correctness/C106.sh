#!/bin/bash

# Invalid: this appends text to an existing array element.
items=(one)
items+=" two"

# Invalid: declaration-created arrays still need +=(...) for new elements.
declare -a flags=(--first)
flags+=" ${extra}"

# Valid: this appends a new element.
items+=("three")

# Valid: scalar append is intentional.
name=base
name+=" suffix"

# Valid: appending to a specific element is outside this rule.
items[0]+=" tail"

# Valid: array defined later should not classify this as an array append.
late+=" value"
late=(value)

# Valid: local scalar shadowing should not inherit outer array type.
arr=(one)
f() {
  local arr=base
  arr+=" two"
}
