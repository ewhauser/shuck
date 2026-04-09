#!/bin/sh

select choice in one two three; do
  printf '%s\n' "$choice"
  break
done
