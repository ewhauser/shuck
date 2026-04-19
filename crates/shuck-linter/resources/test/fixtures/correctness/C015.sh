#!/bin/bash

# Invalid: the redirection still happens before sudo elevates
sudo echo hi > out.txt

# Invalid: appending has the same problem
sudo printf '%s\n' ok >> log.txt

# Invalid: stdin redirections still happen before sudo elevates
sudo cat < input.txt

# Invalid: tee only makes the write privileged; input redirects still happen locally
sudo tee /tmp/out.txt < input.txt >/dev/null

# Valid: move the redirection into the elevated shell
sudo sh -c 'echo hi > out.txt'

# Valid: the tee handoff performs the privileged write
printf '%s\n' hi | sudo tee /tmp/out.txt >/dev/null

# Valid: explicit file descriptors stay outside this compatibility rule
sudo printf '%s\n' ok 1> out.txt 2>> err.log 0< input.txt

# Valid: /dev/null sinks should stay quiet
sudo -u "$user" printf '%s\n' ok 2>/dev/null
sudo cat < /dev/null

# Valid: here-strings are not output file redirects
sudo scutil <<< "ComputerName: example"
