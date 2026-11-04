#!/usr/bin/env bash
# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

set -euo pipefail
# These two installed commands must exit before reading credentials or starting OAuth.
export-tb --help
xero-tb-auth --help
