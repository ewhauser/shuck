#!/bin/sh
set -eu

seconds=${1:?Usage: with_timeout.sh <seconds> <command> [args...]}
shift

state_dir=$(mktemp -d "${TMPDIR:-/tmp}/shuck-bench-timeout.XXXXXX")
timed_out_file="$state_dir/timed_out"

"$@" &
pid=$!

(
    sleep "$seconds"
    if kill -0 "$pid" 2>/dev/null; then
        : >"$timed_out_file"
        kill "$pid" 2>/dev/null || true
        sleep 1
        kill -9 "$pid" 2>/dev/null || true
    fi
) &
watchdog=$!

status=0
if ! wait "$pid"; then
    status=$?
fi

kill "$watchdog" 2>/dev/null || true
wait "$watchdog" 2>/dev/null || true

if [ -f "$timed_out_file" ]; then
    rm -rf "$state_dir"
    exit 124
fi

rm -rf "$state_dir"
exit "$status"
