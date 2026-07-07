#!/usr/bin/env bash

set -euo pipefail

python="$1"

exec "$python" -I -S -c '
import json
import sys

executable = None
for line in sys.stdin:
    message = json.loads(line)
    if message["reason"] == "compiler-message":
        rendered = message["message"].get("rendered")
        if rendered:
            print(rendered, file=sys.stderr, end="")
    elif (
        message["reason"] == "compiler-artifact"
        and message["target"]["name"] == "serc"
        and "bin" in message["target"]["kind"]
        and message["executable"] is not None
    ):
        executable = message["executable"]

if executable is None:
    raise SystemExit("cargo did not report the Serc executable")
print(executable)
'
