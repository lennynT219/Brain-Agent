# H1 bound project-memory archive (provisional)

This is a **library primitive**, not a connector, general Brain note format, YAML
reader, search engine, permission service or automatic transcript archive. Std
file locking requires Rust >=1.89; this locked tree is tested only with 1.100.0.
Existing std/serde/toml/tempfile/uuid dependencies and the CLI/config/document
model are unchanged. All initial use/tests are synthetic.

## Trusted binding and API

A trusted caller supplies an absolute existing root and a typed, valid
`DocumentMetadata::Project` whose UUID equals the supplied project UUID.
`Binding::new` validates these structural conditions, not consent, real-vault
existence, repository mapping or the caller's authority. UUIDs, paths and markers
are selectors, **not authorization**. No HOME/config discovery or vault scanning
occurs. Callers must enforce the confirmed project grant outside model-controlled
arguments. Personal/other-project access requires separate explicit direction.

One fixed `stela-shell` collection per project is shared by authorized sessions.
Session identity is provenance, not private ownership or memory lifetime. Source
references are inert attribution, not fetched evidence, instructions or approval.
Missing source revisions/locators stay explicitly absent; attribution is not
verification and claim kinds do not establish truth/confidence.

```rust,ignore
let binding = Binding::new(root, &typed_project, project_id)?;
let store = MemoryStore::new(binding);
store.create_from_view(memory_id, metadata)?; // UUID/view supplied by caller
store.read_current(memory_id, max_bytes)?;
store.read_revision(memory_id, revision, max_bytes)?;
store.publish_successor(memory_id, expected_revision, new_bytes, metadata)?;
```

The caller first creates the collection/memory directories and editable `view.md`.
H1 snapshots this explicitly scoped input. It does not author a Markdown note,
create project bindings, generate UUIDs, import manual changes or merge branches.
Duplicate creation returns `AlreadyExists` (or a corruption error), never resets
history. General request-key/exactly-once delivery guarantees are **deferred**.
No operation automatically retries, rebases, repairs or deletes revisions.

## Layout and raw-byte format

```text
<root>/<project-uuid>/stela-shell/
  .writer.lock
  <memory-uuid>/
    view.md
    revisions/<positive-decimal-number>.bmr
```

The caller owns the editable view. Brain creates machine scaffolding/records only.
Each BMR1 record is eight bytes `BRAINMR1`, a four-byte little-endian unsigned
header length, a UTF-8 TOML `Header`, then exactly `payload_len` raw bytes. This
internal envelope is **not** TOML note storage or a preservation serializer for
`DocumentMetadata`. BOM, CRLF, comments, unknown YAML properties and body bytes
are opaque and retained exactly; even non-UTF-8 payloads are preserved.

Header identity includes store version 1, project/collection/memory UUID scope,
revision, optional parent, `source_view = "view.md"`, `base_revision = 1`, payload
length and title/claim/source/session/observation metadata. `store_version` and
`brain_version` are different schema domains, neither a content freshness token.
Revisions are positive numeric IDs up to `i64::MAX`, not timestamps or digests.
All store writers must respect immutable records; there is no tamper protection
or authenticated history. Arbitrary editors must edit the separate view, not the
revision store. Intermediate external edits Brain never observes cannot be saved.

## Logical heads, view state and historical reads

Revision 1 is the sole root. Every later parent must exist in the same scoped
memory and have a lower revision. This rules out dangling/cyclic/cross-memory
ancestry; each chain must reach the root. New revisions use checked max+1 while
a unique head exists. Multiple maximal heads produce `NeedsReview`; never choose
by number, filename time or session. Valid branches remain intact and readable.
There is no merge API in H1. Supersession is derived ancestry, not deletion or proof
that the earlier claim was false.

Compare `view.md` with **revision 1**, its own immutable base, not the latest
inferred payload. Equal base with head 1 is `Base`; equal base with later heads is
`Stale`; changed bytes are `Diverged`. No automatic view refresh occurs. Current
reads/updates reject divergent views; explicit revision reads remain available
with `Historical`/`Unresolved`/`Current`, heads, view and review status. Missing,
unsafe or unreadable views currently produce an error even for historical reads.
Historical reads during ordinary divergence/competing heads are supported.
Results return total byte count and explicit truncation. "Current" denotes the
resolved archive head, **not** verified source applicability or real-world truth.

## Coordination, publication and failure semantics

Every operation takes a nonblocking exclusive std file lock on the persistent
collection `.writer.lock`; `Busy` does not retry. Never unlink that inode.
Explicit RAII unlock precedes close (also prevents fork-inherited descriptors
from temporarily extending a finished operation's lock). Locks coordinate only
participating Brain processes, not arbitrary editors or hostile filesystem races.
Ancestors must be stable/trusted; observed symlinks/traversal/nonregular leaves
are rejected, but these checks are not a hostile-same-user sandbox or CAS.

Writes stage a complete record, flush/sync it, then use tempfile no-clobber
publication. **Namespace publication is the commit point.** Directory sync and a
second view observation follow. There is no multi-file transaction or promised
power-loss durability; the view is never overwritten.

- Pre-publication validation/write failures publish nothing; machine directories
  or a persistent lock may remain. Drop cleans this operation's temporary file on
  a best-effort basis. No existing revision/view is removed.
- Destination collision returns `Collision` without replacing the destination.
- An ambiguous persist failure returns `PublicationUncertain { revision, .. }`:
  inspect history, do not assume rollback or blindly retry.
- Successful publication followed by sync/observation failure returns
  `AfterPublication { revision, .. }`: the record definitely remains published.
- Observed post-publication view drift returns a published result with
  `needs_review`, never a fabricated rollback. Observation is not atomic with
  arbitrary edits; a later read may detect drift not seen during publication.

A valid published but unacknowledged record participates in future head selection
(the valid-orphan/ack-loss case); it is never garbage-collected here. A dangling
parent is corruption, not an adoptable orphan. Isolated `.pending-*` crash remnants
are ignored, counted against directory bounds and never adopted/deleted by readers.

## Finite limits and validation

Header <=16 KiB; payload/view <=1 MiB; full graph raw records <=8 MiB; <=128
revisions; <=144 directory entries including isolated staging remnants. Publication
reserves room for its new entry. Each requested result <=64 KiB (zero is valid for
metadata-only reads). Graphs/headers are validated before accepting a current
head; graph loading itself is bounded, not a truncated partial-consistency check.
Metadata: <=32 sources, each nonempty supplied string/title/session <=2048 bytes;
observation milliseconds must fit `i64::MAX`. Serialized headers must also fit the
header limit. CPU and allocator overhead are not claimed to equal raw-byte limits.

Errors distinguish binding/input, missing/unsafe resources, busy, duplicate create,
collision, stale revision, review, malformed/unsupported formats, finite limits,
overflow and pre/uncertain/post-publication I/O. Revisions at the limit cannot be
incremented. Corrupt records cannot be skipped to advertise a false clean head.

## Evidence and remaining scope

Synthetic checks live in `tests/memory_store.rs` and private publication tests in
`src/memory/store.rs`. Tests cover raw preservation/history/own-base drift, scoped
validation, numeric/graph/read bounds, IPC-coordinated processes, persistent lock
identity, competing branches, collision and injected pre/post/uncertain failures.
Fault injection tests error semantics, not actual power-loss behavior. Linux-only
operation is observed; other filesystems/platforms, network FS, hostile mutation
and crash durability are not verified. The ignored integration child is explicitly
executed by the process test, not omitted coordination coverage.

H1 does include logical update; it does not complete the general memory product.
Find/search, real-vault verification, the separate YAML reader milestone, managed
Note schema extensions, ID generation, exactly-once requests, manual import/merge,
CLI/skill/StelaShell connector, automatic deletion/capture and services are absent.
StelaShell retains FARO/session/helper lifecycles and its compact WORK_STATE; Git,
original sources, tests and user decisions remain authoritative in their domains.
