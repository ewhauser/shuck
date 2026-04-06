#!/bin/sh

read line
IFS= read name
read -p 'Name? ' answer
read -u 3 fd_line

read -r raw
read -sr secret
