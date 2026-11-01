#!/usr/bin/env bash
# One action of a parallel task in tests/executor.rs. It records how many
# actions of the task run at the same time.
#
# Usage: concurrency.sh <dir> <name> <wait_for>
#
# While it runs, the action keeps the file <dir>/running.<name>. At the
# start, it appends the number of `running.*` files to <dir>/peaks. This
# number is never larger than the number of actions that really run at the
# same time.
#
# If <wait_for> is larger than 0, the action waits until <wait_for> actions
# started, for at most 5 seconds, and fails if fewer started. It succeeds
# only if <wait_for> actions run at the same time. If <wait_for> is 0, the
# action sleeps 0.2 seconds, so that other actions can start in that time.
#
# At the end, the action appends <name> to <dir>/order, and prints <name>.
set -eu

dir="$1"
name="$2"
wait_for="$3"

touch "$dir/running.$name" "$dir/started.$name"
running=("$dir"/running.*)
echo "${#running[@]}" >>"$dir/peaks"

if [ "$wait_for" -gt 0 ]; then
  for _ in $(seq 50); do
    started=("$dir"/started.*)
    [ "${#started[@]}" -ge "$wait_for" ] && break
    sleep 0.1
  done
  started=("$dir"/started.*)
  if [ "${#started[@]}" -lt "$wait_for" ]; then
    echo "only ${#started[@]} of $wait_for actions started" >&2
    exit 1
  fi
else
  sleep 0.2
fi

echo "$name" >>"$dir/order"
rm "$dir/running.$name"
printf %s "$name"
