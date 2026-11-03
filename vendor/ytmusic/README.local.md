Vendored from https://github.com/sonorahq/ytmusic-rs at a8fb6f570abc828b3e899a112f9c8dfbb2e17439 (GPL-3.0-or-later).

Local changes in src/client.rs expose full cookie-store restore/export and a synchronous persistence callback after response cookie changes. The app supplies Windows DPAPI storage; no plaintext cookie file is configured.

Session persistence includes unexpired session cookies, not just cookies with explicit expiry dates. Cookie authorization emits SAPISIDHASH, SAPISID1PHASH, and SAPISID3PHASH for the request origin. Authenticated HTTP 200 guest responses are reported as expired sessions rather than empty libraries.
