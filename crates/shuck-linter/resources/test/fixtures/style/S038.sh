#!/bin/sh
f() {
  echo hello >/dev/null
  return $?
}
f
