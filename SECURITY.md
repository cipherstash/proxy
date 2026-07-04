# Security Policy

CipherStash takes the security of our software, infrastructure, and customers extremely seriously.  
This document describes the security posture, reporting process, and guidelines for the Proxy repository.

## Supported Software

This repository contains the source code for CipherStash Proxy, including:

- The CipherStash Proxy binary 
- Docker containers and Docker Compose configuration
- Encryption migration tool

### CipherStash Proxy

**Security fixes are released for the latest release line.** Security reports are welcome for any version, but fixes land in the latest release — if you are running an older version, plan to upgrade to receive them.


All software follows semantic versioning and undergoes internal security review, automated analysis, and reproducible builds as part of our SDLC.

---

## Reporting a Vulnerability

If you believe you have found a security vulnerability in any CipherStash code, service, or dependency:

📧 **Please email: `security@cipherstash.com`**

We request that you **do not publicly disclose** the issue before we have had a chance to investigate and provide a fix.

When reporting, please include (as applicable):

- Description of the vulnerability
- Steps to reproduce
- Impact assessment or potential misuse
- Any relevant logs, PoCs, or screenshots
- Suggested remediation (if you have one)

We will acknowledge receipt within **48 hours** and provide regular updates until the issue is resolved.

---

## Disclosure & Response Policy

CipherStash follows a **coordinated responsible disclosure** process:

1. **Submit report** privately via `security@cipherstash.com`.  
2. **Acknowledgement** within 48 hours.  
3. **Assessment** of severity using CVSS and internal risk models.  
4. **Fix development** and patch release in a private branch.  
5. **Coordinated disclosure**, including:
   - New patch release(s)
   - Security advisory on GitHub  
   - Credit to reporter (optional)

We will never take legal action against good-faith security researchers who follow this policy.

---

## Scope

The following are **in scope**:

- The `cipherstash/proxy` GitHub repository
- All published Docker images published to [Docker Hub under `cipherstash/proxy`](https://hub.docker.com/r/cipherstash/proxy)
- Proxy cryptographic implementations, configuration layers, and CLI tooling
- Key-handling, authenticated encryption behaviour, JSON/JSONB field-level encryption flows
- Documentation or code examples that could lead to insecure usage
- CipherStash’s internal infrastructure  
- CipherStash CTS, ZeroKMS, or other backend products

The following are **out of scope**:

- Example [schema](./docs/sql/schema-example.sql) and [configuration](./cipherstash-proxy-example.toml) (though we are still grateful for any relevant disclosures there)
- Social engineering, physical attacks, or denial-of-service  
- Attacks requiring privileged access to developer machines or CI/CD infrastructure  

---

## Security Guidelines for Contributors

To maintain a strong security posture, contributors MUST:

### ⚙️ Follow cryptographic safety rules
- Do **not** modify cryptographic primitives without prior discussion 
- Avoid introducing new crypto dependencies without prior discussion  
- Never check in test keys, secrets, or example credentials  

### 🛡 Coding & dependency hygiene
- Avoid adding dependencies unless necessary  
- Keep dependencies updated and vetted  
- Use Rust for all new code  
- Ensure all code paths that handle keys or encrypted data include type-safe boundaries  

### 🔍 Testing & review
- Submit PRs with tests covering edge cases and misuse-resistant behaviour  
- Flag any changes involving key derivation, key wrapping, AAD, or encryption modes for mandatory security review  
- Do not merge PRs that downgrade security controls or introduce unsafe defaults

---

## Questions?

For general questions about CipherStash security practices (not security incidents), contact:

📧 **support@cipherstash.com**

For vulnerability disclosures:

📧 **security@cipherstash.com**

---

Thank you for helping keep Proxy and the wider CipherStash ecosystem secure.
