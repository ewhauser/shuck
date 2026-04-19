#!/bin/bash

printf '%s\n' $(printf '%s\n' 'a b')
printf '%s\n' prefix$(printf '%s\n' stamp)suffix

printf '%s\n' "$(printf '%s\n' 'a b')"
stamp=$(printf '%s\n' now)
