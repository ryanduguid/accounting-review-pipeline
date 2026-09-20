# Security policy

## Supported versions

Security fixes are applied to the latest version on the default branch (`main`).

## Reporting a vulnerability

Please use this repository's private vulnerability-reporting feature on GitHub. Do not open a public issue for a suspected security vulnerability. Include a clear description, reproduction steps, impact, and any suggested mitigation. Make the reproduction a fabricated one: name the component directory, the command or report step, the synthetic input it ran against, the observed output and the expected output. Never attach a client dataset, a tenant name or a screenshot of client data.

We will acknowledge a valid report within 7 days and will coordinate a fix and disclosure timeline with the reporter.

## Data boundary

The report and semantic model read the fabricated CSVs in `samples/`, or the local export you substitute for them, through Power Query on your own machine. Nothing in this directory calls out to this project, sends telemetry or transmits client data; publishing a report to a Power BI workspace afterwards is your tenant's decision and its own data flow.
