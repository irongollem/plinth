# Minihoard in Plinth — from bash wrapper to clickable library

Minihoard (the sibling CLI at `~/Repositories/minihoard`) browses a
MyMiniFactory account and downloads selections without the browser's
5-simultaneous-downloads ceiling or its missing queue. Inside Plinth it is
currently an easter egg: a whitelisted-subcommand terminal proxy streaming
raw console lines into a scrolling `<pre>`. This plan turns that into a
real UI — the library as clickable, filterable data; selection; a download
queue with per-object progress — WITHOUT moving minihoard's network/auth
machinery into Plinth.

## The boundary decision (pinned)

Plinth keeps shelling out to the `minihoard` binary; the CLI gains a
machine-readable output mode. NOT a crate dependency on `mmf-core`:

- The CLI is already the API owner: auth (OAuth + session cookie),
  Cloudflare TLS-impersonation (`wreq`/BoringSSL), the download manifest.
  A crate dep would drag BoringSSL (CMake + NASM on Windows CI) into
  Plinth's build for zero user-visible gain.
- The easter-egg contract survives: no binary on PATH → no tab. Plinth
  never carries MMF credentials or network code.
- It's the house pattern: Blender scripts talk to Plinth in `TOKEN {json}`
  stdout lines parsed into typed events. Minihoard becomes the third
  citizen of that protocol, not a special case.
- `mmf-core::pipeline` already emits structured Progress events
  (ObjectStart/File/ObjectDone/ObjectFailed) that the CLI flattens into
  console text — the JSON mode is a serializer for events that exist, not
  new plumbing.

## Pinned interfaces

### minihoard CLI v0.4.0 — `--json` (NDJSON, one object per line, stdout)

```text
status --json   -> {"event":"status","version":…,"oauth_ok":bool,
                    "username":…?,"cookie_present":bool,"library_dir":…}
                   (non-mutating; cookie VALIDITY is only provable by a
                    real list call — status reports presence/age only)
list --json     -> one {"event":"entry","id":…,"name":…,"creator":…,
                    "creator_username":…,"source":…,"library_added_at":…,
                    "yearmonth":…?,"tags":[…],"downloaded":bool} per object
                   then {"event":"summary","total":N}
get <ids> --json -y
                -> {"event":"object_start","id":…,"name":…,"index":i,"total":N}
                   {"event":"file_progress","id":…,"bytes_done":…,"bytes_total":…}
                   {"event":"object_done","id":…,"dir":…}
                   {"event":"object_failed","id":…,"reason":…}
                   {"event":"job_done","ok":n,"failed":m}
errors          -> {"event":"error","kind":"cookie_expired"|"auth"|…,
                    "message":…} + nonzero exit. Plinth branches on `kind`,
                   NEVER on message wording (the current Vue WIP matches
                   two hardcoded English strings — that coupling dies here).
```

Version gate: Plinth probes `minihoard --version`; < 0.4 keeps the legacy
console UI with an "update minihoard" hint. JSON lines and human output
never mix: `--json` silences the human printer.

### Plinth backend (`minihoard.rs` rewrite, same detection + whitelist)

```text
minihoard_status() -> MinihoardStatus            // probe + auth health
minihoard_list()   -> Vec<MinihoardEntry>        // buffered; ~4k entries is fine
start_minihoard_download(ids: Vec<u64>) -> job_id
cancel_minihoard_download(job_id)                // kill child, Cancelled event
// events: MinihoardDownloadStatus = Started | ObjectStart | FileProgress
//   | ObjectDone | ObjectFailed | Finished | Failed | Cancelled
//   — shaped like BaseCutStatus; user cancel is Cancelled, never Failed.
```

One run at a time (existing ACTIVE_RUN mutex). The raw stdout tail ring
buffer stays for post-mortems. Cookie expiry = typed error kind → the
banner + one-click `sync-cookie` (the WIP's intent, minus string matching).

### UI (Minihoard.vue rewrite)

- Header: account chip (username), auth status, Sync cookie, Refresh.
- Filters: search, creator, month, source, "not downloaded" toggle —
  all client-side over the buffered list.
- Rows: checkbox · name · creator · month · source badge · downloaded ✓.
  Plain filtered v-for with incremental "show more" paging; no
  virtualization until it hurts.
- Selection bar: "N selected → Download" → queue panel with per-object
  rows + aggregate progress + cancel.
- The raw console demotes to a collapsed "log" details panel (debug view).

## Phases

1. **CLI `--json`** (minihoard repo, v0.4.0): serialize existing Progress
   events + entry/status/error shapes above; tests pin every payload.
   _Done when_: `list --json | head` parses, `get --json` streams events,
   human output unchanged without the flag.
2. **Typed backend**: minihoard.rs commands + events + version gate,
   riding the existing spawn/line-reader plumbing. _Done when_: a
   harness-started download emits the full event sequence and cancel
   kills the child.
3. **Clickable library**: the Vue rewrite above. _Done when_: browse →
   filter → select → download → progress → done, no console needed.
4. **Hand-off + polish**: after a download run finishes, offer "scan into
   catalog" (add/reuse the library dir as a catalog root + start scan —
   closes the manual hand-off the current copy apologizes for); proactive
   status check on tab mount; "since last visit" badge off
   `library_added_at` later.

## Risks

- **Version skew**: the JSON protocol is the contract; gate on version,
  keep payloads additive-only after 0.4.0.
- **objectPreviews has no pagination** — every list call fetches the whole
  library JSON server-side. Fine at ~4k entries; don't poll it.
- **No thumbnails in the listing payload**: images need per-object API
  calls — lazy, on-demand, later; never an eager 4k-call sweep.
