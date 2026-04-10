#!/bin/sh
# shellcheck disable=3043,2034,2155
myfunc() {
  local iface="${1}" ifvar="$(echo "${iface}" | tr - _)"
  echo "$iface $ifvar"
}
myfunc eth0
