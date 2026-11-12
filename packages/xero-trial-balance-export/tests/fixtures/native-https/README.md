These public, fabricated TLS fixtures exercise the native HTTP client through an owned loopback CONNECT proxy. The proxy never forwards traffic. Each test child trusts this CA through its own `REQUESTS_CA_BUNDLE`; system trust is untouched.

The leaf covers `api.xero.com` and `identity.xero.com` so the production hostname checks run. Its private key is deliberately public test data. OpenSSL 3.5.9 generated the RSA 2048-bit CA and leaf on 11 November 2026; both expire on 8 November 2036. Regenerate both certificates and the leaf key together before expiry.

The DER certificate and key are stored as base64 text so the repository fixture screen can inspect every input. The test decodes the same bytes with the existing base64 dependency.
