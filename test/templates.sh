#!/bin/bash
# Runs the equivalence test on every fetched template (see fetch_templates.sh) in parallel.
cd "$(dirname "$0")"
for d in templates/*/; do
  node equivalence.mjs "$d" "$(cat "$d/MAIN")" &
done
wait
