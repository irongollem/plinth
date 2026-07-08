# Releasing with Plinth — a guide for creators

This is the creator-facing companion to the [`.3pk` format spec](3PK.md).
It explains what Plinth produces when you pack a release, why it's split
into pieces, and what that buys your customers — especially when you ship
a fixed or extended version of a release later.

## What a packed release looks like

Packing a release in Plinth produces a folder — that folder is the
distributable unit:

```text
My Studio-05-2026-Dungeon Classics/
├── release.3pk          # small: metadata, previews, licence
├── galeb duhr.zip       # one component per model/group
├── giant badger.zip
└── behir.zip
```

- **`release.3pk`** is small and carries everything _about_ the release:
  names, poses, scale, support status, tags, per-file pose assignments,
  preview images, licence files — plus a checksum for every component
  archive and every file inside them.
- **Each component archive** holds the actual model files for one model
  (or one group, when you grouped models in the builder).

Upload the whole folder wherever you distribute files (MMF, Patreon, your
own storage). Keep the sibling zips next to the `.3pk` — it references
them by filename.

## Why modular instead of one big archive?

1. **Updates cost what changed, not the whole release.** Every component
   has its own checksum. When a user opens your v2 `.3pk`, Plinth diffs
   those checksums against what they already imported and offers only the
   changed models — a fixed knight in a 40 GB release means re-importing
   one zip, not forty gigabytes.
2. **Integrity per piece.** Each component verifies independently. One
   corrupted download refuses cleanly while the rest of the release
   imports; the user re-downloads one zip.
3. **Users can import selectively.** Someone who only wants two models
   out of twelve can tick exactly those.

## Shipping an update (v2, fixes, added models)

Just pack the release again from the builder — same release name,
designer, and date — and distribute the new folder. There is nothing
special to do:

- **Fixed/changed models** get new component checksums; your users'
  Plinth flags them as _UPDATE_ and pre-selects them.
- **New models** appear as _NEW_.
- **Untouched models** read as _UNCHANGED_ and are skipped — even if you
  re-upload every zip, users don't re-import them.
- **Removed files** inside a changed component are cleaned up on the
  user's disk automatically.
- Files your users edited locally (e.g. supports saved from their slicer)
  are never overwritten — Plinth keeps their copy aside as
  `name (edited).stl`.

The release name + designer + date is the identity an update is matched
on, so keep those stable between versions. Use the release _version_
field to communicate the revision.

## Deduplication — repeated parts ship once

Sculpts often share parts: one base or body repeated across five weapon
variants. Plinth stores those bytes **once** per component and lists
every filename in the manifest with its checksum. On import, the
duplicates are rematerialized — hardlinked where the user's disk supports
it, copied otherwise — so every expected file exists, but your archive
(and your users' bandwidth) only paid for unique content.

You don't configure any of this; it happens at pack time.

## What your metadata buys your users

Raw STLs carry no metadata. The `.3pk` manifest is how your curation
survives the trip: model names, poses, scale, support status, tags, and
per-file pose assignments all arrive intact in the user's catalog, and
even survive their own rescans. The more you fill in while building the
release, the better it lands.

## Not a distribution format: `model.plinthpack`

If you use Plinth's compressed-at-rest feature, your own library will
contain `model.plinthpack` archives (with `pack.json` sidecars). Those
are **internal storage**, not a distribution format: only your own
catalog knows how to read them, and unlike the frozen `.3pk` spec they
may change between app versions. Anything that leaves your machine —
to customers, or to your own second system — should leave through the
release builder as a `.3pk`. A model that is packed at rest has no
loose files to stage, so unpack it first (Plinth skips packed models
when adding to a release and tells you why).

## Practical tips

- **Group variants of one model** in the builder — they become one
  component, which is the unit users select and the unit dedup works
  within.
- **Include pre-supported files** alongside raw ones; Plinth records
  support status per file and slicer-ready files are preferred at print
  time.
- **Add preview images.** They travel inside `release.3pk` and become the
  catalog cards your release is browsed by.
- **Don't rename the component zips** after packing — the manifest
  references them by filename.
- **Don't hand-edit files inside the folder** after packing — checksums
  are of exact bytes, and a mismatch makes the component refuse to
  import (that's the feature).
