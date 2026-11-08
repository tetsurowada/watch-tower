#!/bin/sh
# Forwards a Claude Code hook payload (stdin) to watch-tower on the host,
# tagged with this container's ID. Never fails the hook.
id=$(sed -n 's#.*/docker/containers/\([0-9a-f]\{64\}\)/hostname.*#\1#p' /proc/self/mountinfo 2>/dev/null | head -n 1)
curl -s --max-time 5 \
  -H "X-Container-Id: ${id:-${HOSTNAME:-}}" \
  -H 'Content-Type: application/json' \
  --data-binary @- \
  "${WATCH_TOWER_URL:-http://host.docker.internal:4567}/hooks/claude" >/dev/null 2>&1 || true
exit 0
