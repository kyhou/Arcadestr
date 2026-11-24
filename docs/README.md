# Documentation and Repository Contents

## Current References

- [CODEBASE.md](../CODEBASE.md): implementation and architecture map.
- [COMMANDS.md](../COMMANDS.md): portable development commands.
- [AGENTS.md](../AGENTS.md): repository rules.
- [Revised screen inventory](../Arcadestr_UI_Screen_Inventory_Revised.md):
  maintained acquisition-aware product requirements.
- [Design handoff](design-handoff/README.md): intentional visual reference bundle;
  [follow-ups](design-handoff/POST_MIGRATION_FOLLOWUPS.md) record remaining work.
- [Regression scripts](../scripts/README.md) and
  [crossflow acceptance](../tests/gate7_crossflow/README.md): test setup and scope.

Root [ADP-01.md](../ADP-01.md), [NIP-102.md](../NIP-102.md), and
[NIP-103.md](../NIP-103.md) are protocol references. `ADP-01.md` is the single
authoritative ADP specification; its former amendments are superseded records in
[protocol-history/](protocol-history/README.md).

**This repository's `ADP-01.md` is the canonical source.** The sibling
`../adp-server/ADP-01.md` is a byte-for-byte mirror, maintained by the process in
`../adp-server/docs/README.md` §Protocol Synchronization:

1. Make protocol changes here only, and commit them.
2. Copy unchanged to `../adp-server/ADP-01.md` and update
   `../adp-server/docs/ADP-01.sync` with this repository's commit and the hash.
3. Run `../adp-server/scripts/check-adp-sync.sh`; it must pass.
4. Record new conformance gaps in each repository's
   `docs/protocol-implementation-gaps.md`.

`NIP-102.md` and `NIP-103.md` are independent specifications, mirrored only by
explicit decision. adp-server CI (`adp-spec-sync`) fetches this repository's
`ADP-01.md` from GitHub: it fails on an edited or unfaithful mirror and warns on
sync lag. Push this repository's spec changes so that check can see them. Client conformance gaps are tracked in
[protocol-implementation-gaps.md](protocol-implementation-gaps.md). They remain
open until code changes; synchronizing documentation does not resolve them.

## Planning and Historical Material

`archive/` preserves original client integration designs, the gated implementation
prompt, the original screen inventory, and the Nostr Connect implementation
report. These are historical context, not current agent instructions or proof
that a feature is complete. Their original text is retained to avoid losing
requirements; repository-root paths in that text retain their original meaning.

`superpowers/plans/` and `superpowers/specs/` contain dated planning snapshots.
Check them against current code and tests before treating them as an active
backlog. The root UI migration/audit documents and the design handoff also
describe distinct design iterations; retain unresolved requirements until their
status has been reviewed.

## What Belongs in Git

Keep source, manifests, lockfiles (including standalone test crates), migrations,
CI, portable shared editor configuration, maintained documentation, and required
test fixtures. In particular, `tests/fixtures/a3-launch-profiles/generated/`
contains checked-in adversarial archives read directly by desktop tests. Keep
the generator and checksums with them; regeneration is not a prerequisite for
running those tests from a checkout.

Keep credentials, real environment files, databases, uploads, build output,
caches, backups, personal editor/agent state, and absolute cross-repository
documentation links out of Git. Shared configuration such as `opencode.json`
and selected `.vscode/` files is intentional when portable and secret-free.

Use the ignored `.local-artifacts/` directory for imported design ZIPs,
one-off screenshots, and document exports. Maintained design references belong
under `docs/` with a documented purpose, rather than as loose root artifacts.
The local `adp-server_CODEBASE.md` link is optional and ignored; the portable
cross-repository reference is `../adp-server/CODEBASE.md`.
