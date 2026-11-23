# Security Policy

## Scope & Threat Model

**Tenerus** is a local-first educational desktop application with **no backend servers, no telemetry, and no brokerage connectivity**. Security controls focus on protecting user-provided API keys, local research files, external feed handling, and the build/release supply chain:

- **Secret handling:** Provider API keys and custom headers are never written to `settings.json`, exported reports, or CLI process arguments. On macOS, secrets can optionally be stored in the system Keychain (`Tenerus inference`); on Windows, Linux, and FreeBSD, secrets remain strictly in process memory for the active session.
- **Network transport:** All remote HTTP requests require **HTTPS** (cleartext HTTP is permitted only for explicit loopback addresses: `localhost`, `127.0.0.1`, `::1`, which automatically bypass system HTTP proxies via `noproxy = "*"`). Request headers, credentials, and payloads are piped to the OS-trusted `curl` binary over `stdin` (`--disable --globoff --config -`) with explicit `proto` restrictions, 4 MB response caps, no redirects on authenticated requests, and error-message secret redaction.
- **Local CLI briefings:** Optional assistant briefings (`codex`, `claude`, `agy`) execute with a cleared environment (`env_clear()` + minimal allowlist), isolated working directory, dedicated Unix process group, strict output size caps (2 MB), and timeouts.
- **Local data isolation:** State, watchlists, imported closes, and prediction logs are written atomically using temporary files (`0600` on Unix) inside user-private data directories (`0700` on Unix). External news links are validated (`http`/`https` with no embedded credentials) before opening.
- **Release supply chain:** GitHub Actions release workflows run with deny-by-default token permissions (`permissions: {}`), `persist-credentials: false`, Node.js 24 actions pinned to immutable commit SHAs, `--locked` Cargo builds against exact dependency versions in `Cargo.lock`, OS-signed compiler packages (no unverified `curl | sh`), and unified `SHA256SUMS.txt` checksums on every release.

## Verifying Downloads

Every GitHub Release includes a `SHA256SUMS.txt` manifest. Always verify downloaded packages before running them:

```sh
# macOS / Linux
shasum -a 256 --ignore-missing -c SHA256SUMS.txt

# FreeBSD
sha256 -c SHA256SUMS.txt

# Windows (PowerShell)
Get-FileHash .\Tenerus-<version>-windows-x86_64.zip -Algorithm SHA256
```

## Reporting a Vulnerability

If you discover a security vulnerability in Tenerus or its release packaging:

1. **Please do not open a public issue** with exploit details.
2. Report it privately through **GitHub Security Advisories** (`Security` → `Advisories` → `Report a vulnerability` on `https://github.com/bnelabs/tenerus`).
3. Include a clear description, affected version/platform, and minimal reproduction steps.

We aim to acknowledge reports promptly and coordinate fixes before public disclosure.
