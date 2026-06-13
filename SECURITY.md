# Security policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately through GitHub's
[private vulnerability reporting](https://github.com/kassian-dev/ironstate/security/advisories/new).
**Do not open a public issue or pull request** for a suspected vulnerability.

We aim to acknowledge a report within five business days, and will keep you
informed as we confirm the issue, prepare a fix, and agree on a disclosure
timeline. Fixes ship as a new patch release, and we credit the reporter unless
you ask us not to.

## Supported versions

The family is pre-1.0 and the crates version together during `0.x`. Security
fixes land on the latest released `0.x` line only; older versions are not
backported. This policy will state a longer support window once `1.0` ships.

| Version        | Supported |
|----------------|-----------|
| latest `0.1.x` | ✅        |
| older          | ❌        |

## What we consider a vulnerability

ironstate exists to make certain properties hold by construction, so a way to
violate any of these is a security issue — not only classic memory unsafety:

- **Hostile-input safety.** Any input to `restore_versioned` (the versioned
  `{version, payload}` decode path — the one place untrusted bytes enter) that
  causes a panic, a hang, or unsoundness instead of a typed `RestoreError`. This
  path is fuzzed on every pull request.
- **Determinism.** Any way to make the same `(seed, journal)` produce a
  different `Digest128` or `AuditDigest` on a supported target, or to draw
  uncounted entropy — anything that breaks the commit–reveal and audit guarantee.
- **Redaction.** Any path by which one principal can observe another principal's
  concealed state through a generated view — a confidentiality leak.
- **Unsoundness** reachable from safe code, including in derive-generated code.

Behavior that is merely incorrect, with no safety, determinism, or
confidentiality impact, should be filed as a normal public issue instead.
