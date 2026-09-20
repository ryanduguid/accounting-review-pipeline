# Security policy

## Supported versions

Security fixes are applied to the latest version on the default branch.

## Reporting a vulnerability

Please use this repository's private vulnerability-reporting feature. Do not
open a public issue for a suspected security vulnerability. Include a clear
description, reproduction steps, impact, and any suggested mitigation.
Make the reproduction a fabricated one: name the component directory, the
command or workbook step, the synthetic input it ran against, the observed
output and the expected output. Never attach a client workbook, workpaper or
screenshot of client data.

We will acknowledge a valid report within 7 days and will coordinate a fix
and disclosure timeline with the reporter.

## Data boundary

The Power Query and VBA in this adapter read the trial-balance CSV you point
them at, on the machine running Excel, and write their output into that
workbook. They have no network call of any kind, so they send no workbook, no
ledger and no client figure anywhere.
