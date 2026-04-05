#!/bin/sh

# Invalid: the redirection still happens before sudo elevates
sudo echo hi > out.txt

# Invalid: appending has the same problem
sudo printf '%s\n' ok >> log.txt

# Valid: move the redirection into the elevated shell
sudo sh -c 'echo hi > out.txt'
