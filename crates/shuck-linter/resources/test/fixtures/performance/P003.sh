#!/bin/sh
if (test -f /etc/passwd); then :; fi
if (test -f /etc/passwd) >/dev/null 2>&1; then :; fi
if ! (test -f /etc/passwd); then :; fi
if ( ! test -f /etc/passwd ); then :; fi
while ([ -f /etc/passwd ]); do :; done
while ! ([ -f /etc/passwd ]); do :; done
until (test -f /etc/passwd); do :; done
until ! (test -f /etc/passwd); do :; done
