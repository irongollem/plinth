# Distributing & moving your library

Models leave a Plinth catalog for two legitimate reasons, and the app
is built around exactly those two:

- **Distributing as a creator** — you made the models (or hold the
  rights to publish them) and want them to reach your customers or
  patrons with your curation intact.
- **Moving your own collection** — the same person, a different
  machine: a new PC, a new NAS, a backup you want to be able to trust.

::: warning Respect model licences
Plinth packs and moves files; it doesn't grant rights. Most purchased
or Patreon models are licensed for personal use only — moving them
between **your own** systems is exactly that, but passing them to
someone else usually isn't. The distribution features exist for
creators publishing their own work.
:::

::: tip Not the distribution format
You may see `model.plinthpack` files inside your library. That's the
space-saving "packed at rest" storage, internal to your catalog — a
model that's packed has no loose files to stage, so unpack it before
building a release (Plinth skips packed models and tells you why).
:::

## Distributing as a creator

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

Publish that folder wherever you distribute your work — your store,
Patreon, cloud storage. Keep the component archives next to the
`.3pk`; the manifest references them by filename. Identical files
repeated across variants are stored once per component and restored on
import, so the archives only carry unique content.

If you distribute at scale, read the [creator guide](../CREATORS.md) —
it covers versioning, how updates reach your users, and practical tips
for structuring releases.

## Moving your collection to another system

Your library is plain folders — Plinth catalogs files where they are,
it doesn't lock them into a database. That gives you two migration
paths:

### Copy the folders (whole-library moves)

1. Run **Clean up…** on the catalog first. Besides normalizing the
   folder structure, the cleaner writes an authoritative `model.json`
   sidecar into every model folder — your names, poses, variants, and
   support assignments recorded _next to the files_.
2. Copy the library folder(s) to the new system (or just point the new
   machine at the same NAS share).
3. Add the folder as a catalog folder on the new install and scan. The
   sidecars restore the catalog — no guessing, no re-curating.

### Pack releases (verified, selective moves)

For moving specific releases — or when you want checksums proving the
copy arrived intact — use the same pack/import flow creators use:
build a release from the models, carry the folder over, and import it
on the other side. Every file is verified against its BLAKE3 checksum
on import, so silent corruption in transit is caught instead of
discovered mid-print.

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
