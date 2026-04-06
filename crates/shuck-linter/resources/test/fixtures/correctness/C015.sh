#!/bin/bash

# Invalid: the redirection still happens before sudo elevates
sudo echo hi > out.txt

# Invalid: appending has the same problem
sudo printf '%s\n' ok >> log.txt

# Valid: move the redirection into the elevated shell
sudo sh -c 'echo hi > out.txt'

# Valid: the tee handoff performs the privileged write
printf '%s\n' hi | sudo tee /tmp/out.txt >/dev/null

# Valid: /dev/null sinks should stay quiet
sudo -u "$user" printf '%s\n' ok 2>/dev/null

# Valid: here-strings are not output file redirects
sudo scutil <<< "ComputerName: example"
