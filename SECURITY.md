# Security policy

## Reporting a vulnerability

Please **do not** open a public issue for a security problem.

- Preferred: use GitHub's private advisory flow —
  <https://github.com/ffredyk/nimby/security/advisories/new>
- Otherwise write to **security@ffredyk.cz** with `Nimby` in the subject.

Include the version (`nimby --version`), what you did, what happened, and — if you
can — the repository URL or a minimal `repo.json`/`mod.srf` that reproduces it.

You can expect an acknowledgement within a few days and a fix or a mitigation plan
in the next release. Credit is given in the changelog unless you ask otherwise.

## Supported versions

| Version | Supported |
|---|---|
| 0.9.x | yes |
| older | no |

## What matters most

Nimby downloads and links files that other people publish, and it writes into
folders you point it at. The classes of bug that matter most are:

1. **Path traversal / arbitrary write** — a repository manifest that makes Nimby
   write outside the workspace, the shared store or the temp folder. Manifests are
   untrusted input: file names, mod names and checksum lists come from the server.
2. **Link escapes** — a junction, symlink or hardlink that ends up pointing outside
   the intended folder, or a "link" that silently overwrites a file Nimby did not
   create.
3. **Checksum bypass** — accepting a file whose MD5 does not match the manifest, or
   reusing a cached hash for content that changed (hash keys are path+size+mtime).
4. **Command injection** — anything that reaches a shell, or `nimby launch` building
   a command line with unquoted user input (server addresses, `-name`, parameters).
5. **Archive/parser problems** — malformed `repo.json`, `mod.srf` or PBO headers
   causing a panic, unbounded allocation or a denial of service.
6. **Credential leaks** — repository credentials must never appear in logs, in
   `.nimby/*.json`, in the tracking index or in error output.

Please report those even if you think the impact is low; they are the areas where a
small bug becomes a serious one.

## Out of scope

- Builds are not code-signed and Windows SmartScreen warns on first run (a known,
  documented limitation).
- Problems that require a malicious local user with write access to the workspace,
  the shared store or the Steam library.
- Vulnerabilities in Arma 3, Steam or the Swifty client itself — report those
  upstream.
- Symlinks requiring Developer Mode or elevation: Nimby prefers junctions and
  hardlinks, which do not.
