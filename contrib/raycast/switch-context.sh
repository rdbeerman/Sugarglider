#!/bin/bash
# @raycast.schemaVersion 1
# @raycast.title Switch Context
# @raycast.mode compact
# @raycast.packageName Sugarglider
# @raycast.argument1 { "type": "text", "placeholder": "Context" }
set -euo pipefail
/usr/local/bin/sugarglider context switch "$1" 2>&1
