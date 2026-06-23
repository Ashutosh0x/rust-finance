# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

**Please do NOT open a public GitHub Issue for security vulnerabilities.**

Instead, please report them through GitHub's private security advisory feature:

1. Go to the [Security Advisories page](https://github.com/Ashutosh0x/rust-finance/security/advisories/new)
2. Click **"New draft security advisory"**
3. Fill in the details of the vulnerability
4. Submit -- only repository maintainers can see this

Alternatively, email **security@ashutosh0x.dev** with:
- Description of the vulnerability
- Steps to reproduce
- Impact assessment
- Any suggested fix

### What to Expect

- **Acknowledgment** within **48 hours**
- **Triage and severity assessment** within **5 business days**
- **Fix or mitigation** within **30 days** for critical issues
- Credit in the release notes (unless you prefer anonymity)

### Scope

The following are in scope:
- The `rust-finance` binary and all workspace crates
- GitHub Actions workflows and CI/CD configuration
- API key handling and secret management
- WebSocket connection security (Alpaca, Binance, Finnhub)
- EIP-712 signing implementation (Polymarket)
- Any dependency with a known CVE that affects this project

### Out of Scope

- Third-party API provider vulnerabilities (Alpaca, Finnhub, etc.)
- Social engineering attacks
- Denial of service against public APIs
