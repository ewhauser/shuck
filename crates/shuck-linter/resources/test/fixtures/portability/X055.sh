#!/bin/sh

# Should trigger: locale-translated dollar-double-quoted string
echo $"Usage: $0 {start|stop}"

# Should trigger: dollar-double-quoted string in a larger word
printf '%s\n' prefix$"translated"suffix

# Should not trigger: plain double-quoted strings remain portable
echo "Usage: $0 {start|stop}"
