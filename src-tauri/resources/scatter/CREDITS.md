# Credits — Plinth bundled scatter assets

Every piece in this bundle is CC0 (public domain dedication) EXCEPT the
organic-leaf set, which is CC BY-SA 4.0 and DOES carry a legal attribution
and share-alike obligation — see the "Printables — organic leaf set"
section below. For the CC0 pieces no attribution is owed, but crediting the
source institutions/authors is good practice and is recorded here per
docs/SCATTER.md's "in-app credits panel + CREDITS file when attribution
is owed" convention (owed or not).

## Smithsonian Institution — National Museum of Natural History

Source: Smithsonian Open Access (`api.si.edu/openaccess`), 3D
photogrammetry/CT scans. Every record below is machine-tagged
`"metadata_usage": { "access": "CC0" }` by the Smithsonian's own API —
the same CC0 designation used site-wide since the Smithsonian's Open
Access initiative launched February 2020. Full license text and
per-file provenance: `smithsonian-3d/LICENSE.txt` in the curation
folder.

- **Hesperocyon skull** (`skull-hesperocyon.stl`) — NMNH Paleobiology.
  <http://n2t.net/ark:/65665/3c6da70bf-20aa-4906-9593-6b7e7f162f9c>
- **Pseudocynodictis skull** (`skull-pseudocynodictis.stl`) — NMNH
  Paleobiology.
  <http://n2t.net/ark:/65665/39df7f8a3-b7fe-40de-91f0-fa361dca8173>
- **Leptophoca lenis (seal) skull** (`skull-leptophoca-seal.stl`) — NMNH
  Paleobiology.
  <http://n2t.net/ark:/65665/3741ffec8-61d7-4af4-af46-d681d2c5a871>
- **White-tailed deer skull** (`skull-deer.stl`) — NMNH Education &
  Outreach. <http://n2t.net/ark:/65665/3c5df1823-689c-44bd-8ca5-4c748c234ea2>
- **Diplocaulus magnicornis skull** (`skull-diplocaulus.stl`) — NMNH
  Paleobiology.
  <http://n2t.net/ark:/65665/37c497201-5f67-4f76-8459-39ad82a45682>
- **White-tailed deer mandible** (`bone-deer-mandible.stl`) — NMNH
  Education & Outreach (same specimen record as the deer skull, above).
- **White-tailed deer forelimb bone** (`bone-deer-forelimb.stl`) — NMNH
  Education & Outreach (same specimen record as the deer skull, above).
- **Pilot whale (Globicephala melas melas) mandible**
  (`bone-pilot-whale-mandible.stl`) — NMNH Vertebrate Zoology, Mammals
  Division.
  <http://n2t.net/ark:/65665/3dd176d5a-0c9d-4259-bdc8-ccde6b2b9fe6>

Suggested credit line: "3D model courtesy of the Smithsonian Institution,
National Museum of Natural History, Smithsonian Open Access (CC0)."

## OpenGameArt

- **Mushroom** (`mushroom.stl`) — Author: JeremyWoods. License: CC0.
  Source: <https://opengameart.org/content/mushroom-3>. Sourced from
  the author's sculpted `Mushroom.blend` (not the flat low-poly game
  mesh) — this is the piece that survived the "next to a sculpted mini"
  eyeball pass documented in docs/SCATTER-ASSETS.md's curation verdict.

Suggested credit line: "Mushroom model by JeremyWoods (OpenGameArt.org),
CC0."

## Poly Haven — forest-floor scans

These two photogrammetry-derived meshes are published under CC0 by
Poly Haven. Attribution is not legally required, but the creator details
and original asset pages are preserved here:

- **Broken forest branch** (`forest-branch-scan.stl`) — source asset
  `dead_quiver_branch_01`. Photography: Greg Zaal. Modeling: Jenelle van
  Heerden. <https://polyhaven.com/a/dead_quiver_branch_01>
- **Fallen forest log** (`forest-log-scan.stl`) — source asset
  `dead_tree_trunk`. Author: Rob Tuytel.
  <https://polyhaven.com/a/dead_tree_trunk>

The bundle contains geometry only—no Poly Haven textures. The source
meshes were scaled to 28–32mm tabletop use, voxel-remeshed into one closed
shell, stripped of floating scan fragments, and capped at 15,000 triangles.
The reproducible conversion script is `tools/curate_polyhaven_forest.py`.

## Printables — organic leaf set (CC BY-SA 4.0)

The five leaf pieces are derived from the "Organic Leaves Set (realistic
look)" published on Printables under **CC BY-SA 4.0**. This is the only
non-CC0 asset in the bundle: attribution is legally required, and any work
that incorporates these leaves (a decorated base, an exported/printed
model) is itself CC BY-SA 4.0 (attribution + share-alike).

- **Maple / Apple / Cherry / Oak / Hazel leaf**
  (`leaf-maple.stl`, `leaf-apple.stl`, `leaf-cherry.stl`, `leaf-oak.stl`,
  `leaf-hazel.stl`) — source: "Organic Leaves Set (realistic look)".
  Author: credited on the source page linked below (the specific username
  was not recorded at curation time; attribution is via the canonical
  Printables link, which names the creator).
  <https://www.printables.com/model/324354-organic-leaves-set-realistic-look>
  License: CC BY-SA 4.0 <https://creativecommons.org/licenses/by-sa/4.0/>

Changes made: decimated to ~1,500 triangles, XY-normalized to a ~5mm
footprint with the blade thickness set to 1.2mm (the ~90mm print original
would downscale to unprintable foil, and a thin leaf vanishes into the
scatter's 0.4mm stitch-sink; 1.2mm is deliberately chunky so it prints and
sits proud), and a gentle curl baked in so the leaves cup rather than lie
flat. The reproducible conversion script is `tools/curate_leaves.py`.

## What did NOT make the bundle

Two Smithsonian resource-tier downloads (Merycoidodon sp. skull,
Diictodon feliceps skull) arrived mesh-corrupted at the served
resolution and were excluded per `smithsonian-3d/LICENSE.txt` — not
processed, not shipped. See the per-piece admission table in the S4a
curation report for the full reasoning.
