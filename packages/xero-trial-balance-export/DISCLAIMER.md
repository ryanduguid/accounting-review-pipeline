# Disclaimer

xero-trial-balance-export pulls a read-only trial balance from the Xero
Accounting API and writes it to CSV. It does not provide professional advice.

Xero report shapes, OAuth scopes and API behaviour change, and a run
reflects only the ledger at the moment it was fetched. Reconcile every export
against the source ledger before relying on it, and keep professional
judgement with a qualified practitioner.

The tool requests the read-only `accounting.reports.trialbalance.read` scope,
so it cannot write to any ledger. It does not post journals, lodge a BAS or a
tax return, lock a period, make a payment, or send client correspondence.
Those remain authorised human actions.

The local environment file and the token cache both hold credentials. Treat
them like passwords, keep the clone inside your own user profile, and never
commit them. Do not publish client trial balances, private tax records, TFNs,
Medicare numbers, bank details, identity documents, or other sensitive
personal information in issues, pull requests, examples, tests, or repository
content. The sample output in this repository is fabricated.

See [LICENSE](LICENSE) for copyright and [SECURITY.md](SECURITY.md) for
vulnerability reporting.
