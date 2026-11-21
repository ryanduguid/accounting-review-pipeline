#!/usr/bin/env bash
# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.

set -euo pipefail
# These two installed commands must exit before reading credentials or starting OAuth.
export-tb --help
xero-tb-auth --help
