#!/usr/bin/env bash
set -euo pipefail
# These two installed commands must exit before reading credentials or starting OAuth.
export-tb --help
xero-tb-auth --help
