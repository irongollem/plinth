# Sharing releases

Sharing in Plinth always goes through the **release builder**: models
leave your machine as a `.3pk` release — checksum-verified,
deduplicated, and carrying your curation — never as loose files or
internal archives. This page covers both directions: packing a release
and importing one.

::: tip Not the sharing format
You may see `model.plinthpack` files inside your own library. That's
the space-saving "packed at rest" storage, internal to your catalog —
don't send those to anyone. If a model you want to share is packed,
unpack it first; Plinth skips packed models when adding to a release
and tells you why.
:::

## Packing a release

1. **Collect models** — in the Catalog, open a model's drawer and hit
   **+ Add to release**. A group goes in with all its poses; per-file
   pose assignments ride along.
2. **Fill in the details** — in the **Releases** tab, the draft stepper
   asks for the release info: designer, release name, date (defaults to
   the current month), description, images. Fields like your designer
   name can be remembered for future releases. Drafts survive an app
   restart — an unfinished release offers to resume.
3. **Pack** — finalizing stages everything into the canonical layout,
   writes a `model.json` sidecar per model, compresses each group into
   its own component archive, and emits `release.3pk` with a manifest
   listing a BLAKE3 checksum for every archive and every file.

The result is a folder:

```text
My Studio-05-2026-Dungeon Classics/
├── release.3pk          # manifest + previews + licence
├── galeb duhr.zip       # one component per model/group
├── giant badger.zip
└── behir.zip
```

Share that folder however you like — cloud storage, Patreon, a USB
stick. Keep the component archives next to the `.3pk`; the manifest
references them by filename. Identical files repeated across variants
are stored once per component and restored on import, so the archives
only carry unique content.

If you distribute releases at scale (customers, patrons), read the
[creator guide](../CREATORS.md) — it covers versioning, how updates
reach your users, and practical tips.

## Importing a release

Drag a `release.3pk` onto the Plinth window (or double-click it — the
installer registers the file type). Plinth reads the manifest and shows
an import dialog listing every component with its file count and size.
Pick what you want — everything is pre-selected on a first import — and
confirm. Every archive is verified against its checksum before anything
lands in your library; a corrupted download refuses cleanly, per
component, while the rest imports.

The release lands in your first catalog folder under
`Designer/YYYY-MM Release/`, a scan picks it up automatically, and the
creator's curation — names, poses, scale, support status, previews,
tags — appears in your catalog as if you'd entered it yourself.

## Updating a release you already have

Open a newer `.3pk` of a release you've imported before and Plinth
diffs it against your library, per component:

| Badge         | Meaning                                             |
| ------------- | --------------------------------------------------- |
| **NEW**       | not in your library yet — pre-selected              |
| **UPDATE**    | changed since your import — pre-selected            |
| **UNCHANGED** | identical — skipped (re-tick to repair lost files)  |
| **PACKED**    | packed at rest locally — unpack it first            |
| **MISSING**   | its archive isn't next to the `.3pk` — can't import |

Only what you select is rewritten, so a fixed model in a huge release
means re-importing one component, not the whole thing. Two safety rules
apply during an update:

- **Your edits are never overwritten.** A file you changed since the
  import — say, supports saved from your slicer — is kept aside as
  `name (edited).stl` before the new version lands.
- **Leftovers are cleaned, additions are kept.** Files the new version
  dropped are removed; files you added yourself are untouched.
