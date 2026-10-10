#!/bin/sh
for arg in "$@"; do
    if [ "$arg" = "--build" ] || [ "$arg" = "-E" ]; then
        exec /opt/homebrew/bin/cmake "$@"
    fi
done
exec /opt/homebrew/bin/cmake -DCMAKE_POLICY_VERSION_MINIMUM=3.5 "$@"
