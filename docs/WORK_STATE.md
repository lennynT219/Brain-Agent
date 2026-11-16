# Work state

## Objective and scope

- H1: library-only bound-project memory archive, including create-from-view, revision/current reads, immutable logical successors and stale-update rejection. Implemented with isolated synthetic fixtures; awaiting parent-arranged read-only review.
- Authorized surfaces: `src/lib.rs`, new `src/memory/{mod,store}.rs`, `tests/memory_store.rs`, `docs/PROJECT_MEMORY_STORE.md`, this record. Brain writer owns these; StelaShell parent owns its own documents/integration.
- Preserve the pre-existing document/CLI/config/vault/Cargo/lock/features/tests implementation; no dependency changes/downloads, real-data activation, profiles/services, branch/switch, push/publication or external/model operations are authorized. The user authorized one local commit of the existing Brain base plus H1; the personal `Brain/` vault and `.codegraph/` are excluded.

## Current decisions

- Brain is the general second brain; project memory is one capability. One stable StelaShell collection per project is shared across authorized bound sessions; sessions are provenance, not ownership/lifetime.
- Automatic management applies only to that project's agent-owned memories. Personal notes/other projects require explicit direction; recalled text is data, not permissions/instructions. No automatic transcript capture/deletion.
- Selected update strategy: new write-once revisions, separate human-editable view, no automatic same-note replacement or manual import/merge.
- The user directly confirmed deleting the untracked `odd/` records and instructed leaving them absent; do not restore/reconstruct them. This resolves the unexplained-deletion blocker, not the pending H1 review.
- The user confirmed one local commit containing the Brain base plus H1: code, repository configuration, Cargo/lock, tests and documentation. Exclude `Brain/` and `.codegraph/`; leave untracked `odd/` deleted. No push or further commits are authorized by that decision. Git history supplies the commit identity.
- H1 takes a trusted supplied typed Project and caller-supplied UUIDs/pre-existing view. Structural validation does not prove consent, repository mapping or actual vault-project existence. No discovery/generator/YAML reader/connector.
- Revision store assumes cooperating writers and trusted stable ancestors; exact-byte snapshots plus numeric identity are not tamper-proof history or filesystem CAS. Original view base is revision 1, not the newest inference.

## Outcome status

- H1 implemented and scoped checks passing; source is review-ready, independent review still pending. The missing `odd/` records are confirmed intentional by the user and remain absent. Six allowed H1 source/test/document paths only; no full memory-product completion claimed.
- Typed managed metadata (older file-model block 1) remains intact. File-model block 2/YAML reader remains separately blocked pending its own dependency/workload authorization; H1 does not resume it.
- Existing init/config behavior preserved by unchanged source plus isolated CLI regressions.

## Relevant evidence

- Branch `feat/brain-file-model`; H1 start baseline was `5ccb129fdc817ccc0c64ad4b593431167a1d3cbf`. Before the requested commit, all 41 remaining protected pre-existing files were byte-identical; H1 changed only the existing library export and its five new files. Five baseline `odd/tasks/brain-{init-tests,vault-bootstrap,human-names,vault-config,file-model}.md` files were intentionally deleted by the user; they were untracked and cannot produce a Git deletion. No restoration is intended.
- Linux x86_64, rustc/cargo 1.100.0. Existing locked dependencies available offline; Cargo/lock/features unchanged.
- `cargo test --locked --offline --test memory_store`: 10 passed; 1 ignored IPC child entry is explicitly executed and checked by the deterministic subprocess test.
- `cargo test --locked --offline --lib`: 12 passed (7 existing metadata, 5 private publication/lock tests). `--test cli_init --test cli_config`: 10 + 5 passed. Scoped rustfmt check passed; no warnings in final checks.
- Earlier parallel tests caught lock lifetime extension through fork-inherited descriptors; explicit RAII unlock corrected it, with a deterministic cloned-descriptor regression test and passing parallel/process checks.
- Format, API, finite bounds, publication/uncertain-I/O semantics, failure injection and platform limits are detailed in `docs/PROJECT_MEMORY_STORE.md`. Fault injection is not power-loss verification; no Windows/network-FS/full-suite/clippy/crash-durability claim.

## Next actions and open questions

- Keep the personal `Brain/` vault and `.codegraph/` outside the authorized code/base/H1 commit. Do not interpret the local commit request as push/publication or broader feature permission.
- Parent to arrange read-only H1 review; transport acceptance is not completion/approval. No further feature expansion or real-vault activation authorized. Leave the user-deleted `odd/` absent.
- General request-key/exactly-once guarantees, ID generation, real-vault project verification, managed-Note extensions, lexical search, manual import/reconciliation and StelaShell connector remain outside H1.
- All revision writers must honor write-once storage. Arbitrary-editor edits never observed cannot be retained. Missing/unsafe/unreadable views currently block even historical reads; ordinary manual divergence/competing-head history reads are supported with explicit status.
