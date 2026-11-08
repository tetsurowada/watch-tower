#!/bin/sh
# Forwards a Claude Code hook payload (stdin) to watch-tower on the host.
# Adds this container's ID and the session's latest title from the transcript.
# Never fails the hook.
payload=$(cat)

id=$(sed -n 's#.*/docker/containers/\([0-9a-f]\{64\}\)/hostname.*#\1#p' /proc/self/mountinfo 2>/dev/null | head -n 1)

# A manual title (custom-title) wins over the generated one (ai-title).
# The raw transcript line goes base64-encoded; watch-tower parses it.
title_line=""
transcript=$(printf '%s' "$payload" | sed -n 's/.*"transcript_path" *: *"\([^"]*\)".*/\1/p' | head -n 1)
if [ -n "$transcript" ] && [ -r "$transcript" ]; then
  title_line=$(grep -a '"type":"custom-title"' "$transcript" | tail -n 1)
  [ -z "$title_line" ] && title_line=$(grep -a '"type":"ai-title"' "$transcript" | tail -n 1)
fi
title_b64=$(printf '%s' "$title_line" | base64 | tr -d '\n')

printf '%s' "$payload" | curl -s --max-time 3 \
  -H "X-Container-Id: ${id:-${HOSTNAME:-}}" \
  -H "X-Session-Title-Line: $title_b64" \
  -H 'Content-Type: application/json' \
  --data-binary @- \
  "${WATCH_TOWER_URL:-http://host.docker.internal:4567}/hooks/claude" >/dev/null 2>&1
exit 0
