#!/bin/sh
trap 'echo caught signal' 1 2 13 15
trap -- '' 0
trap -p 2
trap '' HUP
command trap '' 9
