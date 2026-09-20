# Security policy

## Supported versions

Security fixes are applied to the latest version on the default branch.

## Reporting a vulnerability

Please use this repository's private vulnerability-reporting feature. Do not
open a public issue for a suspected security vulnerability. Include a clear
description, reproduction steps, impact, and any suggested mitigation.
Make the reproduction a fabricated one: name the component directory, the
command, the synthetic input it ran against, the observed output and the
expected output. Never attach a client export, a tenant name, a token or a
screenshot of client data.

We will acknowledge a valid report within 7 days and will coordinate a fix
and disclosure timeline with the reporter.

## Data boundary

This component reads Xero's Trial Balance report over the Xero API, using the
credentials and token cache named in `.env.example`, and writes the CSV and its
manifest to the path you choose. It sends your data nowhere else: there is no
telemetry, no analytics and no upload, and a bug report must carry fabricated
figures rather than an export.
