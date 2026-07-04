# `.3pk` Release Format — Specification v1

Plinth packs a release as a **`.3pk` manifest archive** plus **sibling
component archives** holding the heavy model files. The `.3pk` is the
portable source of truth for everything the catalog knows about a release:
raw STLs carry no metadata, so the manifest is what lets one user's
curation — names, poses, scale, supports, tags, and per-file pose
assignments — survive intact when another user scans the same release.

This document freezes **format version 1**. The manifest carries
`"version": 1`; readers reject unknown majors and ignore unknown fields
within a known major (forward-compatible).

## Physical layout

A packed release is a directory (the distributable unit):

```
Designer-05-2026-Dungeon Classics/
├── release.3pk              # zip: manifest.json + images/ + licence/
├── galeb duhr.zip           # component archive (one per group/model)
├── giant badger.zip
└── behir.zip
```

- **`release.3pk`** — a zip archive containing `manifest.json`, the
  release-level preview `images/`, and any `licence/` + PDFs. Small; this
  is what a client fetches first to learn the release's shape.
- **Component archives** (`<component>.zip`, later `<component>.tar.zst`)
  — the model files (STL/OBJ/3MF/LYS/…) for one group/model. One archive
  per component so a client can fetch, verify, or update them
  independently (the Modular Package Strategy: selective download and
  update-detection fall straight out of the per-component checksums).

The `.3pk` never embeds the heavy model files — only references them by
archive filename + checksum. A single-file distribution is a future
option; v1 is deliberately modular.

## `manifest.json` schema (v1)

```jsonc
{
  "format": "3pk",
  "version": 1,
  "generator": "plinth/<app-version>",

  "release": {
    "name": "Dungeon Classics",
    "designer": "Some Designer",
    "date": "2026-05", // YYYY-MM (canonical; input MM/YYYY is normalized)
    "version": "1.0.0",
    "description": "…",
    "tags": ["dungeon", "classic"],
    "images": ["images/cover.png"], // paths inside release.3pk
  },

  "components": [
    {
      "name": "galeb duhr", // the logical model / group
      "archive": "galeb duhr.zip", // sibling file, relative to the release dir
      "checksum": "blake3:9f86d0…", // of the archive bytes — drives update detection
      "size_bytes": 896812345,

      "models": [
        {
          "id": "0e37…-uuid", // stable identity; survives moves/rescans
          "name": "galeb duhr", // scanner/base name
          "custom_name": null, // user override, if any (else null)
          "description": null,
          "group": "galeb duhr",
          "tags": ["earth-elemental"],

          "designer": "Some Studio", // the studio/brand (defaults from release, per-model override)
          "sculptor": "A. Artist", // the individual artist, if known

          "pose": "A", // model-level metadata (→ model_user_meta)
          "scale": "32mm",
          "support_status": "unsupported",
          "release_date": "2026-05",
          "preview": "images/galeb duhr A.png",

          "files": [
            // paths are relative to the component archive root
            {
              "name": "A/body.stl",
              "checksum": "blake3:…",
              "size_bytes": 1234,
              "pose": "A",
              "support_status": "unsupported",
            },
            {
              "name": "shared/base.stl",
              "checksum": "blake3:…",
              "size_bytes": 567,
              "pose": null,
              "support_status": null,
            },
          ],
        },
      ],
    },
  ],
}
```

### Field → catalog mapping

The manifest is the wire form of the catalog's rescan-safe tables. On
import the scanner restores:

| Manifest field                                                                       | Catalog destination                             |
| ------------------------------------------------------------------------------------ | ----------------------------------------------- |
| `models[].custom_name`, `pose`, `scale`, `support_status`, `release_date`, `preview` | `model_user_meta` (overrides scanner inference) |
| `models[].tags`                                                                      | `model_tags` (source `metadata`)                |
| `models[].files[].pose` / `support_status`                                           | `file_variants` (the dump-folder splits)        |
| `release.*`, `components[]`                                                          | `models.release_name/designer`, group identity  |

Because `file_variants` rides in the manifest, a folder someone split into
poses on their machine reappears already split on yours — the whole point
of making pose _metadata_ rather than folder structure.

### Checksums

- **Algorithm:** BLAKE3 (already shipped for duplicate detection), encoded
  `blake3:<hex>`.
- **Component `checksum`** (required): hash of the archive file's bytes.
  This is what update-detection compares — a changed component is a
  changed hash, so a client re-fetches only what moved.
- **File `checksum`** (recommended): hash of each model file's content,
  for granular integrity and cross-release dedup.

## Write path (packer)

At pack time the builder already stages models with their catalog
metadata. Packing then:

1. Groups staged models into components (by `group`, else per model).
2. Writes each component's model files into `<component>.zip`, hashing the
   archive → `checksum`, and each file → per-file `checksum`.
3. Emits `manifest.json` from the staged metadata **including
   `file_variants`** for any split folders, plus release-level info.
4. Zips `manifest.json` + release images + licence into `release.3pk`.

Compression is ZIP in v1 (the only writer today); TAR+Zstd is a tracked
upgrade and only changes component `archive` extensions + the reader's
dispatch, not the manifest schema.

## Read path (scanner import)

When a scan encounters a `release.3pk`:

1. Read `manifest.json`; reject unknown `version` majors.
2. For each component, index its models exactly as folder-scanned models,
   but seed identity from the manifest (`id`, `name`, `group`).
3. Restore user metadata into `model_user_meta`, tags into `model_tags`,
   and per-file assignments into `file_variants` — all keyed by
   `dir_path`/`path` so they survive later rescans like any local edit.
4. Verify component `checksum` when present; a mismatch surfaces as a
   health warning rather than blocking the scan.

Legacy `release.json` / `model.json` sidecars remain readable; the `.3pk`
manifest supersedes them when both are present.

## Versioning & compatibility

- `version` is a single integer. Same-major additions are additive fields
  (ignored by older readers); a breaking change bumps the major and
  readers refuse to guess.
- There is **no live release yet**, so v1 is defined freely here — no
  migration from a prior on-disk format is owed.

## Out of scope for v1 (tracked separately)

- Single self-contained `.3pk` container (v1 is modular).
- Selective/partial download + reconstruction UI (builds on component
  checksums — see the Modular Package Strategy todo section).
- TAR+Zstd component compression (todo: replace ZIP for local
  compression/cataloging).
- Compress-at-rest for catalog models (uses the same component archives).
