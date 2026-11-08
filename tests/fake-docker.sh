#!/bin/sh
# Stands in for `docker inspect --type container --format <fmt> <id>` in tests.
# Knows one container, whose ID starts with "aaaa".
id=""
for arg in "$@"; do id=$arg; done
case "$id" in
  aaaa*)
    printf '%s\t%s\n' \
      aaaa000000000000000000000000000000000000000000000000000000000001 \
      /Users/me/src/api
    ;;
  *)
    echo "Error: No such container: $id" >&2
    exit 1
    ;;
esac
