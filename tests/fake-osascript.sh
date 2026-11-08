#!/bin/sh
# Stands in for osascript in tests.
# argv: -e <script> <message> <title> <subtitle> <sound>
# Writes argv, NUL-separated, to $FAKE_OSASCRIPT_OUT when it is set.
case "$3" in
  __fail__)
    echo "fake failure" >&2
    exit 1
    ;;
  __hang__)
    exec sleep 30
    ;;
esac
if [ -n "$FAKE_OSASCRIPT_OUT" ]; then
  # Write then rename so readers never see a partial file.
  printf '%s\0' "$@" > "$FAKE_OSASCRIPT_OUT.tmp"
  mv "$FAKE_OSASCRIPT_OUT.tmp" "$FAKE_OSASCRIPT_OUT"
fi
