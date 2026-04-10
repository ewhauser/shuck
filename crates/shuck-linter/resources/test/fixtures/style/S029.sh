#!/bin/bash
echo HEAD@{1}
echo @{1}
eval command sudo \"\${sudo_args[@]}\"
echo [0-9a-f]{$HASHLEN}
find . -exec echo {} \;
if [[ "$hash" =~ ^[a-f0-9]{40}$ ]]; then
  :
fi

echo "HEAD@{1}"
echo x{a,b}y
