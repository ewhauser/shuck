#!/bin/sh

# Should trigger: unquoted escaped pipe/brace sequences in echo arguments
echo usage: cmd [start\|stop\|restart]
echo token\{on,off\}

# Should not trigger: quoted arguments and non-echo commands
echo "usage: cmd [start\|stop\|restart]"
echo 'token\{on,off\}'
printf '%s\n' usage: cmd [start\|stop\|restart]
echo plain | pipeline
