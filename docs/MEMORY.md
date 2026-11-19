# Memory core and local JSON CLI

## Scope

Brain implements a fourth document model, **Memory**: persistent working context,
not each message/fact. A Memory optionally belongs to one existing Project;
a Project can group multiple Memories. Note/Project/Task and `init`/`config`
remain compatible. No Project, Task, index Note, catalog or whole-vault scan is
created automatically.

```text
<selected-vault>/05-memorias/<memory-uuid>/
  Memoria.md       # ordinary current Markdown, not a generated projection
  Historial.md     # complete accepted versions, including latest exact bytes
  .lock           # stable OS-lock file, not owner metadata
  .pending.json   # only while publication is incomplete
  .reassociation.json # retained provenance only for replace_missing
```

05-memorias is lazy, not a new requirement for validating existing vaults. UUID is
in the directory/frontmatter/working binding, not a long display basename.
Actual qualified links connect notes, e.g.
`[[05-memorias/<UUID>/Memoria|Memoria]]` and
`[[05-memorias/<UUID>/Historial|Historial]]`. Project links point to the explicit
Project document **outside 05-memorias**. No Obsidian settings or graph-layout
assumptions are required. Association never changes Memory ID/location.

No StelaShell extension, installer, daemon or HTTP/MCP service is included in
this repository. Brain is the sole primary project-continuity authority; use
the host-selected working-context Memory through the StelaShell integration.
The former `docs/WORK_STATE.md` record was explicitly migrated to that Memory
and retired; do not recreate it as a fallback, synchronized mirror or parallel
continuity store. This migration does not authorize migrating other Memories,
vaults or legacy records.
Brain's explicit operations are turn-agnostic. The host must resolve/read before
work on a genuine human prompt and call create only after a useful successful
response; startup/greetings/empty/failed/cancelled turns are not creation events.
The host enforces permissions and human approval; paths, UUIDs, note links,
configuration and a JSON boolean **do not establish authority**.

## Existing `Memorias/` data is not automatically migrated

The canonical area is now **`05-memorias/`**, including required relations and
history headers. There is no legacy alias, fallback search, auto-move or rewrite.
An unchanged binding still identifies the same UUID, but lookup in the new area
reports `not_found` if that UUID exists only under `Memorias/`. This is **not
proof that the old area's data is absent**; do not use that result alone to
approve `replace_missing` or create a replacement identity.

Moving only the UUID directory is insufficient: the old history header still
links to `Memorias/`, so the new validator reports `partial/history_corrupt`.
Changing only that header also cannot migrate it: accepted snapshots with the
old required Historial relation produce `partial/history_document_invalid`.
Normal update/recovery is not a migration operation and must not rewrite those
accepted bytes to make them pass. Synthetic tests cover both failures and byte
preservation; no actual vault is migrated by this change.

The minimal safe operational choice for existing data is to keep its old binary
and files paired, and validate the new binary on temporary new fixtures. Applying
this version to an existing Memory requires a separate explicit compatibility
or migration decision that accounts for the immutable accepted snapshots, UUID
and binding. No such migration mechanism or approval is implied here.

## Current Markdown model

```yaml
---
brain_type: memory
brain_version: 1
brain_id: "<UUID>"
title: "Working context"
memory_revision: 1
# Optional pair, omitted when unassociated:
# brain_project: "<Project UUID>"
# project_link: "[[01-proyectos/StelaShell|StelaShell]]"
---
```

The body has exactly these canonical H2s, in order:

```markdown
## Objective and scope
## Current decisions
## Outcome status
## Relevant evidence
## Next actions and open questions
```

The qualified history link must be an **active wikilink** in the body; when
associated, the qualified Project link must also be active, not metadata alone.
Literal examples do not qualify: backtick/tilde fences, indented code (including
mixed spaces/tabs), inline code, inline/multiline HTML comments, escaped brackets,
HTML attributes/elements and Markdown destinations/reference definitions.
An active link alongside supported inactive examples does qualify. Creation
inserts the links under Relevant evidence / Current decisions.

`memory::markdown::MemoryDocument` parses real YAML with serde-saphyr, validates
owned typed metadata and retains the original raw document. Unknown fields,
comments, newline styles and body bytes are not reserialized through Serde.
Normal update accepts the complete replacement body and changes only the digits
of the owned revision scalar. This retains **all frontmatter bytes except those
digits**. Revisions start at 1 and use checked u64 increment.

Validation uses pulldown-cmark's CommonMark block/inline events and original
source ranges, with its Obsidian-style wikilink option; it does not render HTML.
Ordinary unordered/ordered lists, checkbox text, quotes, nested structures and
paragraph/lazy continuations are allowed, including genuinely active links in
them. No Brain Task or Project is created from Markdown structure. Fenced,
indented and inline code, HTML comments/attributes/literal elements, destinations,
images, escaped/entity-derived or ambiguous bracket examples do not qualify.
A genuine link alongside inactive examples does qualify. Existing valid snapshots
with ordinary lists/quotes remain readable without rewriting accepted bytes.

Exactly five ordered canonical **top-level** `## <name>` lines are required;
nested headings do not substitute for them. Additional `###` headings are allowed;
they do not replace the canonical H2s. Write placeholder examples such as
`05-memorias/<UUID>/Memoria.md` and `ss_<area>_<action>` in inline code (or
escape their angle brackets). Bare `<UUID>` is interpreted as an HTML element,
not a placeholder, and without a matching closer is rejected. No escaping or
normalization is performed on behalf of the caller.
Unclosed fences, multiline/unclosed
HTML tags and Setext/dash-only separator syntax retain their conservative
rejection policy; use `***` for a divider. A validation-only, equal-byte-length
view interprets bare CR as a newline alongside LF/CRLF; source offsets still
refer to original bytes. Parser-recognized HTML comments are masked in that
validation view through their source-level `-->`, before the shared topology
pass. Only the earliest currently recognized comment is masked, then the parser
rediscovers subsequent contexts on that view until no comments remain. Literal
fake fences therefore cannot hide later genuine comments or expose comment-like
text in later real code. Each round removes an existing opener without creating
one; there is no hardcoded pass count. This may reparse once per comment, so
worst-case work grows with comment count times document length; no performance
bound is claimed. This remains true across quote/list boundaries or when the closer appears
in a Text/Code event or tag attribute: neither links nor H2s inside the comment
count. Real links/headings after closure remain valid. Masking preserves offsets
and newlines and is never stored or used to reserialize content.
Markdown interpretation and exact original-token checks are shared for section
and relation validation; no prefix-based blanket container prohibition remains.
Existing
noncanonical YAML revision spellings (e.g. quoted keys, aliases, nondecimal
scalars) may parse but cannot be silently rewritten: update returns `invalid`
requiring a separately reviewed correction. No content is normalized away.

Normal updates cannot change title/Project/unknown frontmatter as a side effect.
Explicit manual review/import can accept those changes after validation. An
associated Project is checked at its explicit safe vault-relative `.md` path,
including its UUID/type. No other notes or link targets are automatically read.

## Working-folder binding and lazy creation

```toml
binding_version = 1
memory_id = "550e8400-e29b-41d4-a716-446655440000"
memory_subfolders = ["child", "nested/child"] # optional explicit relative paths
```

`.brain.toml` contains no vault/content/history/Project/permissions/executable.
Unknown fields, nil UUIDs, unsupported versions, duplicate declarations,
traversal, absolute paths, glob syntax and nonportable separators are rejected.

`binding::resolve(working_directory, authorized_boundary)` requires existing
directories, walks no higher than the supplied boundary and stops at the nearest
marker. A closer own marker wins over a more distant parent's declarations.
On the nearest marker, its most specific applicable explicit child declaration
selects a separate **unbound** child context; otherwise use the enclosing binding.
No declared child is created eagerly. Explicit child creation materializes its
own marker at the declared directory, not an arbitrary descendant.
Copies/moves of a marker reuse the SAME Memory in the selected vault. No
cross-vault synchronization or broader access follows from an ID.

Missing markers are normal; malformed, inaccessible, symlink/nonregular markers
and traversal below the boundary fail explicitly. This is not a sandbox against
concurrent hostile path replacement; parent directories must be controlled by
the trusted caller. Read/resolve create nothing, including lock files.

Creation uses a short `.brain.toml.lock` OS lock in the actual target directory,
then rechecks resolution. A competing marker is never overwritten. A durable
`.brain.pending.toml` receipt reserves the chosen Memory UUID/request ID before
store publication. It holds no context and is not a permission grant. A retained
receipt blocks blind retry/new UUID allocation and is reported as `partial`.
The stable lock sidecar can remain after failed admission; it is not a Memory.

### Explicit replacement of a genuinely missing reference

Ordinary `create` still never overwrites an existing marker. The separate
library `Store::replace_missing` / CLI `replace_missing` operation requires
`human_approved:true`, the authorized context, actual `binding_directory`, old
`expected_memory_id`, **exact `expected_marker` bytes**, and proposed title/five
sections/optional Project. The host must show the proposed creation and marker
change and approve this exact action/content; a JSON boolean is not authority.

Under the short binding lock, Brain repeats resolution and exact marker checks,
then verifies the old Memory is `not_found` **and its UUID directory is absent**.
Found, partial/corrupt, unavailable, busy, pending, changed identity/bytes,
incorrect target and declared-but-unbound child states do not qualify. Brain
chooses a distinct UUID; it never reuses the old identity or adopts partial data.
Only the approved marker's identity value is changed, preserving all other marker
bytes and `memory_subfolders`. Other copies and old artifacts are not edited.

The reservation/intent capture exact old marker bytes. Before marker replacement,
Brain retains `.reassociation.json` in the new UUID directory with request ID,
old/new IDs and old/new marker strings. It remains after successful cleanup.
Recovery accepts only the exact old or exact already-published new marker; it
never restores a deleted marker or overwrites third-party bytes. Before changing
an old marker, old-directory absence is rechecked. The existing pre-intent
uncertainty rules apply: inspect/report, no blind retry or second UUID.

## Historial framing v1

Header, using LF framing:

```text
# Historial

<!-- brain-history-v1 <canonical-uuid> -->

[[05-memorias/<canonical-uuid>/Memoria|Memoria]]

```

Each complete accepted record:

```text
## Revision <positive-u64>
<!-- snapshot <request-uuid> <UTF-8-byte-count> <fence-length> -->
<fence>markdown
<exact snapshot bytes>
<fence>
<!-- end-snapshot -->

```

The fence is `max(3, longest backtick run in snapshot + 1)` backticks. Parsing
reads the declared UTF-8 byte count, not a delimiter search; the LF before the
closing fence is framing, **not snapshot content**. Arbitrary embedded fences,
Unicode, CRLF, comments and absent final newline are retained exactly.

The parser requires header identity, complete footer/EOF, canonical safe fence,
valid UTF-8 boundaries, contiguous revisions from 1 and unique nonnil request
UUIDs. Empty history is only a construction state. The store additionally
validates every retained snapshot's Memory type/identity/sections/revision and
active required relations. Previously accepted snapshots whose only required
relation is literal are now reported `partial` (`history_document_invalid`),
not rewritten, discarded or silently imported.

Readable non-UTF8 history (including a truncated multibyte sequence) is content
corruption: `partial`, reason `history_corrupt`, never `unavailable/InvalidData`.
Raw bytes remain untouched; no replacement characters are introduced. Missing
history remains `partial/history_missing`. Actual filesystem/type/access errors
remain `unavailable` with their I/O category when available.

History contains complete accepted documents, including latest accepted bytes,
not prompts/drafts or lossy diffs. Prepared candidates live only in temporary
pending intent, not as accepted history. A history replacement may be published
while intent still exists; it is **unconfirmed** until pending state is cleared.

No automatic condensation, 8–16KiB target, retention cap, segmentation,
archival/rotation/deletion, benchmark or growth experiment exists. The current
implementation parses/copies history proportional to retained content and has
ordinary finite memory/disk/parser capacity. Allocation/I/O/parser failures are
not permission to truncate. This is neither unlimited storage nor an approved
future retention policy. History is not authenticated against coordinated edits
to both history and current or valid edits to older historical content.

## Concise change log (history format 2)

An ordinary `create` request with a one-line `summary` (1–1024 UTF-8 bytes,
no control characters) starts a concise `Historial.md`. Existing format-1
histories are not converted by reads, section updates or ordinary saves;
creation without `summary` retains the legacy behavior.

Format 2 shows dated UTC entries, with one short description per accepted
change rather than numbered copies of `Memoria.md`. It is not a means of
restoring past Memory contents. Hidden entry markers correlate the internal
revision and request; the replaceable `.accepted.json` keeps one exact accepted
Memory, bound to the log tip. Keep this sidecar with the two Markdown files.
It does not accumulate copies. It is not a second user-facing Memory or a
distributed lock. The history file is still published through a safe staged
rewrite; the logical change is an appended entry, not a full Memory snapshot.

`update_sections` takes a map of canonical H2 titles to replacement section
contents (headings excluded). It preserves untouched bytes, checks heading
boundaries and validates the complete successor including its required links.
A byte-identical successor adds no entry. As for a full-body update, stale
expected bytes or revision are a conflict, not an implicit merge.

The pending intent for format 2 includes exact before/after accepted-head
bytes in addition to the Markdown files. Publication verifies all three before
removing the pending barrier. Inspection includes optional `accepted_head`;
explicit recovery requires the full unchanged observation. No implicit repair,
retry, pruning or migration.

## Publication, conflicts and manual approval

`Store` opens an explicitly selected valid vault. Current lookup is direct by
UUID. Short per-Memory `File::try_lock` locks serialize cooperating reads and
publication. Contention returns `busy` immediately; there are no waits/retries,
PID/age-based stealing or invented live-owner identities. The stable lock file
is never removed/replaced. OS locks are released when the handle/process exits.
Rust >=1.89 is required for these APIs; actual checks used Rust1.100 on Linux.
Other platforms/filesystems are not runtime-verified.

Update requires both expected accepted revision and exact accepted Markdown.
Stale state returns `conflict`. An identical body returns `unchanged` without
another snapshot. External current changes return `manual_review`, preserving
both files; normal update cannot silently adopt/merge/overwrite them.

`read` returns exact accepted/observed bytes and, when valid, an
`approval_candidate`. The candidate preserves external bytes except revision
digits needed for the next accepted version (or retains already-advanced next
revision). The host must show/approve that exact candidate. `accept_external`
requires the baseline, exact observed bytes, exact approved candidate and an
explicit human-approval decision, then rechecks under lock. Stale approval is
rejected. Missing/corrupt history is `partial`, not a missing Memory or adoption
permission. Invalid external identity/sections/revision has no import candidate.

Publication sequence:

1. Recheck exact current/history bytes under the short lock.
2. Stage and publish a no-clobber `.pending.json` intent containing exact before
   and next document/history, operation ID and optional creation marker bytes.
3. Stage/sync/publish current, then history using same-directory tempfile files.
4. On ordinary creation, publish marker without clobbering another identity;
   on approved replacement, retain provenance and replace only exact old bytes.
5. Verify current/history/marker and receipt ownership, clear creation receipt,
   then clear store intent.
6. Only then return `saved`.

These are **separate writes**, not a transaction, filesystem CAS, tamper-proof
log or power-loss guarantee. The intent contains a recoverable candidate, not
an accepted draft. Any failure after durable intent may be `partial` even when
some desired bytes reached disk. Reads and ordinary updates do not repair it.
A noncooperating editor can race after a comparison; directory/symlink checks
are not a secure openat sandbox. Do not claim arbitrary-editor race protection.

## Explicit recovery and unavoidable incomplete states

`inspect_recovery` returns exact raw current/history/intent and optional
context marker/receipt/reassociation provenance. Invalid UTF-8 history is carried
as exact `history_bytes` (JSON integer-byte array), with `history:null` and
`history_corrupt:true`; inspection reports `partial/history_corrupt`. Valid UTF-8
with invalid history/snapshot structure is also flagged corrupt. It writes
nothing. `recover` rejects corrupt history rather than repairing/adopting it,
including when an intent exists. `recover` requires that complete
observation and explicit human approval, rechecks under locks, validates every
snapshot and the retained prefix, and finishes only the recorded successor.
It accepts current/history only when they match recorded before or next bytes;
other external edits cause conflict. It never rolls back, regenerates arbitrary
current from history, discards accepted versions or chooses a fresh identity.
Creation recovery requires its authorized context and matching receipt or an
already-published exact matching marker. Cleanup interruption remains recoverable
because intent is removed last.

There is an unavoidable earlier window: a creation receipt may be durable before
a candidate intent exists (or before a new directory/lock is fully prepared).
In that case resolve reports partial and creation refuses retry, but `recover`
cannot guess missing content. It returns partial/not_found/unavailable according
to the observed artifacts and preserves them. Explicit further human-directed
reconciliation is required; no automatic allocation, cleanup or repair is
implemented. Similarly, absent/corrupt acceptance baseline without valid intent
cannot be automatically recovered by this API. These cases do not prevent the
host from ordinary work or reporting the problem without Memory persistence.

## JSON CLI protocol v1

```sh
brain memory --max-input-bytes 4194304 --max-output-bytes 4194304
```

One UTF-8 JSON request on stdin to EOF; one JSON response plus LF on stdout.
Both default transport bounds are **4MiB**, configurable from 1KiB to 1GiB.
These are transport safety ceilings, not desired Memory sizes or history
retention limits. Excess input/complete output returns `too_large`; context is
never silently truncated. The output bound includes its final LF. Writes only
return small receipts/errors so a document-size response cannot fail after a
save; host loss of the small receipt can still leave confirmation unknown.

From this checkout (no global installation required), use
`cargo run --locked --offline -- memory` after building dependencies. For a
read-only capability probe, pipe this request to that command:

```json
{"protocol_version":1,"request_id":"550e8400-e29b-41d4-a716-446655440001","operation":"capabilities","arguments":{}}
```

Request envelope (strict fields):

```json
{
  "protocol_version": 1,
  "request_id": "550e8400-e29b-41d4-a716-446655440001",
  "operation": "capabilities",
  "arguments": {}
}
```

An optional absolute `vault` selects the caller-authorized existing vault;
otherwise existing validated Brain configuration is used. Capabilities needs
neither vault nor configuration and never initializes anything. Supported
operation names are at most 32 UTF-8 bytes. Request ID is a nonnil UUID, used
for correlation/accepted-record provenance, not authorization or retry policy.

Responses: `{protocol_version:1, request_id, operation, result}`. Successfully
parsed IDs/operations are echoed. Malformed envelopes use null correlation;
overlong operation names are not echoed. Logical results/errors are JSON with
exit0; transport/process failures may exit nonzero without a confirmed result.
No interactive prompts, shell execution, model call or network service.

| Operation | Strict `arguments` |
| --- | --- |
| capabilities | `{}` |
| resolve | `{working_directory, authorized_boundary}` |
| read | `{memory_id}` |
| create | `{context:{working_directory,authorized_boundary}, title, sections:[five strings], project?, memory_subfolders?:[relative paths], summary?}` |
| replace_missing | `{context, binding_directory, expected_memory_id, expected_marker, human_approved, title, sections:[five strings], project?}` |
| update | `{memory_id, expected:{revision,accepted_markdown}, body}` |
| update_sections | `{memory_id, expected:{revision,accepted_markdown}, changes, summary}` |
| accept_external | `{memory_id, expected:{revision,accepted_markdown}, observed_markdown, approved_markdown, human_approved}` |
| inspect_recovery | `{memory_id, context?}` |
| recover | `{memory_id, context?, observation:<exact inspection observation>, human_approved}` |

Optional `project`: `{brain_project:<UUID>, path:"01-proyectos/StelaShell.md", label:"StelaShell"}`.
`create.sections` is in the five-section order; Memory UUID is generated by Brain
with UUID v4, so the caller does not need to guess its own-history link. Later
`update.body` is the complete body containing the actual qualified relations.

`resolve` reports unbound/partial, or binding plus nested Memory state and the
same top-level Memory status. `read` returns status, ID, accepted revision/bytes,
optional observed bytes/import candidate and diagnostic reason. Inspection
normally returns `{status:"found", observation}`: here found means an observation
was obtained, **not** that a valid accepted Memory exists. Corrupt history instead
returns `{status:"partial",reason:"history_corrupt",observation}`. Observation
contains `memory_id,current,history,pending,marker,reservation` (raw components
nullable), optional `reassociation`, `history_bytes`, `accepted_head`, and
`history_corrupt` (false by default). Pass the entire unchanged observation back;
these are additive v1 fields, not authority. It may contain sensitive context;
do not log it unnecessarily.

### Exact operation arguments

`vault` is optional in the envelope: omit it to use the existing validated
`brain config` selection. `capabilities` needs neither vault nor configuration.
All operation argument objects reject unknown fields.

| Operation | `arguments` |
| --- | --- |
| `capabilities` | `{}` |
| `resolve` | `{working_directory, authorized_boundary}`: existing directory paths |
| `read` | `{memory_id}` |
| `create` | `{context:{working_directory,authorized_boundary}, title, sections:[s1,s2,s3,s4,s5], project?, memory_subfolders?, summary?}`; section strings contain content only, in canonical order; `summary` selects history format 2 |
| `replace_missing` | `{context:{working_directory,authorized_boundary}, binding_directory, expected_memory_id, expected_marker, human_approved, title, sections:[s1,s2,s3,s4,s5], project?}`; exact approved old marker/identity and actual authorized target; existing child declarations preserved, not supplied again |
| `update` | `{memory_id, expected:{revision,accepted_markdown}, body}`: complete successor body, not a full document |
| `update_sections` | `{memory_id, expected:{revision,accepted_markdown}, changes, summary}`: only changed section contents, keyed by canonical H2 |
| `accept_external` | `{memory_id, expected:{revision,accepted_markdown}, observed_markdown, approved_markdown, human_approved}` |
| `inspect_recovery` | `{memory_id, context?}`; authorized context is needed to include creation marker/receipt |
| `recover` | `{memory_id, context?, observation, human_approved}`; pass the complete exact prior observation |

Optional `project` is `{brain_project:<UUID>,path:<vault-relative .md>,label}`.
Creation chooses a new UUID, adds owned metadata/links and returns a receipt;
it never takes a model-supplied identity to overwrite. `body` in update includes
the five H2s and existing qualified links. Normal mutation success is
`{status:"saved",memory_id,revision,request_id}`; replacement also returns
`replaced_memory_id` (the approved old ID). No-op returns `unchanged`.
During recovery the receipt's request ID identifies the original pending
publication; the outer envelope still correlates the current recovery request.
Response envelope is always `{protocol_version:1,request_id,operation,result}`.
Logical outcomes use process exit0; transport/process failures exit nonzero.
Malformed envelopes return null correlation; an operation name longer than32
bytes is rejected without echoing it, preserving bounded output.

Statuses include `found`, `unbound`, `not_found`, `unavailable`, `busy`,
`conflict`, `manual_review`, `partial`, `saved`, `unchanged`, `invalid`,
`unsupported`, `too_large`. A genuinely missing reference is nonblocking for the
host: inform and offer **new** creation/reassociation separately via explicit
`replace_missing`; never silently replace the marker or emulate it in SS by
unlinking/rewriting marker files. Access/vault/I/O failures are not `not_found`. `io_kind`, when
available, preserves the OS error category without leaking context in diagnostics.

Validation failures caused by a literal HTML element still open at end of body,
or a parser-recognized HTML comment without its source-level `-->`, return:

```json
{"status":"invalid","message":"document or arguments failed validation","reason":"UNCLOSED_HTML"}
```

This fixed category uses the **existing optional `Failure.reason`** field in
protocol v1, not a new `code` field. It is derived from a typed syntax error, not
message matching; the diagnostic contains no document fragments, paths or title.
Other rejection categories retain their existing contract (in particular,
malformed/multiline tag syntax and unclosed code fences are not recategorized).
Both `create` and `update` propagate this reason through the ordinary failure
serializer. Rejected create publishes no identity/binding, reservation or Memory
files (an internal candidate UUID may already have been generated); rejected
update preserves the accepted document/history and binding. A host must treat
`invalid` as unsaved and must not infer a binding or silently retry a save.

Source regressions for this diagnostic, placeholder escaping/inline code, H3
acceptance and preservation after invalid update are prepared in
`tests/cli_memory.rs`. They have **not been executed** in this change: tests,
builds, Brain execution and formatting tooling remain outside the authorized
verification scope; evidence here is static review and `git diff --check` only.

### Cancellation and confirmation

The host owns a finite process deadline, stdin EOF, stdout/stderr bounds and
termination of its child process. Brain never polls/retries or waits for locks.
If cancelled before a complete request is admitted, it has not dispatched a
mutation. Termination/disconnection during or after publication may lose the
receipt: the host must report **save_unconfirmed**, not saved/rolled-back, and
must not retry or allocate another UUID. Future read-only resolution/inspection
can reconcile actual state. Brain cannot emit a reliable cancellation response
once killed; no automatic signal-handler rollback is attempted.

`human_approved:true` is an admission assertion from the trusted host, not
cryptographic evidence or permission the model can grant itself. SS must keep
executable/vault/working boundary/identity/operations in authorized host state,
exclude helpers and apply its response-success policy. This CLI implements no
such SS hooks or installer decisions.

## Build and verification

Human-approved dependency changes: serde-saphyr2.1.1 (defaults disabled,
deserialize+serialize), serde_json1.0.154, uuid1.28.0 with serde+v4. Include and
property-resolution features are not enabled. Subsequently, under its own human
authorization, the coordinating session prepared exact pulldown-cmark0.13.6 with
default features disabled and unicase2.10.0, preserving all existing package
entries. No optional Cargo features are enabled for either new package. The
parser declares MSRV1.71.1; unicase declares none. Brain's declared minimum remains
1.89; actual minimum-toolchain execution is not verified. Setup used cached
archives verified against lock checksums and locked/offline metadata; no package
download or Cargo re-resolution was performed by this implementation session.
Tests run `--locked --offline` with an isolated target directory and temporary
synthetic vault/work folders.

- `tests/memory_primitives.rs`: typed model, strict markers, boundaries,
  copies/moves, lazy declarations, exact history framing/fences/UTF-8.
- `tests/memory_documents.rs`: real YAML, unknown/comment preservation,
  CRLF, sections, duplicate keys, revision syntax/overflow.
- `tests/memory_store.rs`: durable snapshots, no-op, manual approval, stale/busy
  writers, concurrent creation, explicit Projects, missing/corrupt history,
  publication fault injection, exact reviewed recovery and cleanup interruptions.
- `tests/cli_memory.rs`: correlation, complete lifecycle, bounds/invalid protocol,
  missing vs unavailable, pre-dispatch cancellation and init/config preservation.
- `tests/memory_corrections.rs`: active versus literal history/Project relations,
  legacy-invalid baseline preservation, approved missing-reference replacement,
  exact marker/copy/declaration preservation, admission and interruption/recovery
  failures, concurrency, non-UTF8 raw-byte corruption and JSON operation receipts.
- `tests/memory_containers.rs`: quoted/nested quote/list fences and indented
  history/Project examples rejected through CLI without a revision or file change;
  genuinely active history/Project links in lists/quotes/checkboxes/nesting and
  continuations; active links alongside inactive examples; old valid list/quote
  snapshots remain found; LF/CRLF/CR, unknown frontmatter and full snapshots remain
  exact. Only previously unsound literal-only baselines become partial.
  Cross-event HTML-comment regressions cover both relations, headings, closers in
  attributes/comments/plain text/code, genuine relations after closure and exact
  LF/CRLF/CR/history preservation. Ordered rediscovery also covers chained
  multiline comments with opaque fake fences and adjacent real code, inline
  code, escaped openers and attributes that only contain comment-shaped text.
  The former blanket-container-rejection
  expectation was replaced by positives.
- The existing R1 relation fixture now separates history and Project paragraphs:
  four-space indentation immediately following paragraph text is a real active
  continuation, not an indented code block. The negative still tests actual
  indented code, and container regressions separately cover the positive case.
- Existing Note/Project/Task/init/config tests remain unchanged.

Fault injection is an explicit library test seam (`Store::with_fault`), never a
CLI option or environment-controlled production behavior. Synthetic tests do not
prove power-loss behavior, arbitrary-editor CAS, every OS/filesystem, or actual
StelaShell settlement/cancellation integration. No personal vault was used.
