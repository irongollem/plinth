# Scatter asset sourcing — research scratchpad

**Date:** 2026-07-15
**Status:** RESEARCH ONLY. Nothing in this document has been downloaded,
opened in Blender, checked for manifoldness/watertightness, or printed.
Every license quote below was read off a page during a web-research pass
and MUST be re-verified on the live page (and ideally re-screenshotted for
the record) at the moment we actually pull a file in, since license terms,
uploader identity, and even whether the file is still hosted can change.
Treat every row as a _lead_, not a cleared asset.

Context: see `docs/SCATTER.md` and `docs/BASECUTTER.md` ("three-source
policy") — this file is scouting for the "bundled" source only. Bundled
means it ships inside the installer, so the bar is: CC0/public domain
(no attribution owed), or CC-BY with attribution light enough for a
credits panel + CREDITS file. NC, ND, SA, "free for personal use," and
any site-custom non-redistribution license are hard no's — decimating,
remeshing, and boolean-unioning the piece into a landscape counts as a
derivative, which ND forbids outright, and SA would try to drag our own
bundle content under its terms.

## Curation verdict (2026-07-15, after the 101-piece preview render)

The downloaded packs were converted and batch-rendered (101/101 clean —
contact sheet at `~/Downloads/plinth-scatter-curation/curation-report.html`),
and the verdict is a hard narrowing: **game assets are for games, not for
printing.** Resin reproduces every facet at 0.05 mm, so a 176-tri prop
that reads fine in a viewport reads as a toy next to a sculpted mini.
The one survivor of the eyeball pass was the OpenGameArt mushroom —
3,584 tris / 175 KB, sourced from an actual sculpted .blend.

That yields a SELECTION CRITERION for organic pieces: binary STL is
~50 bytes/triangle, so **reject anything under ~150 KB (≈3k tris) as
sourced** — below that a piece cannot carry sculpted surface detail —
and admit into the bundle at **≤ ~1 MB after decimation** (the ~15k-tri
target). Size is a proxy for sculpt-density, not a substitute for the
eyeball + manifold gate; it's the cheap first filter. Hard-surface bits
(crates/gravestones/barrels — legitimately angular, legitimately light)
are exempt from the floor but only earn a slot if a subdivide/bevel A/B
render at true base scale passes the same "next to a sculpted mini" test.

Bundled set direction: scan-quality skulls (Dundee/raven remix, manual
download pending) + the OGA mushroom + at most a couple of A/B-approved
hard-surface pieces. Everything else organic goes PROCEDURAL (rocks and
pebbles already shipped in scatter_landscape.py; bones and tufts join as
generated kinds) — zero bundle bytes and no aesthetic clash. Real premium
scatter enters through the user-library door, which is S4's other half.

## Summary

| Category                       | Verdict           | Notes                                                                                                                 |
| ------------------------------ | ----------------- | --------------------------------------------------------------------------------------------------------------------- |
| Skulls (human/animal/monster)  | **Rich**          | University of Dundee Museum Collections CC0 scans are the standout find; several usable directly or via a CC-BY remix |
| Bones (scattered)              | **Poor**          | Hobbyist Thingiverse/MyMiniFactory uploads are NC-SA or paid-non-redistributable; no clean CC0 loose-bone pack found  |
| Rocks / rubble                 | **Good**          | Kenney + Quaternius CC0 game-asset kits are plentiful; need remesh/thicken pass, not print-ready as-is                |
| Mushrooms                      | **Thin but real** | A handful of CC0 low-poly hits (OpenGameArt, Quaternius); no sculpted/high-detail CC0 mushroom found                  |
| Plants / leaves / tufts        | **Weakest**       | Purpose-built wargaming tuft packs are all paid, non-redistributable; only low-poly CC0 game-asset foliage qualifies  |
| Gravestones                    | **Good**          | Kenney Graveyard Kit + a clean individual Sketchfab CC0 tombstone                                                     |
| Barrels / crates / wood debris | **Good**          | Kenney Pirate/Dungeon kits + OpenGameArt "Free Wooden Crates" all CC0                                                 |
| Generic fantasy debris         | **Good**          | Covered by the same Kenney dungeon/tiny-dungeon kits                                                                  |

Weak spots, as predicted going in: **bones** and **plants/tufts**. For
both, procedural generation (a handful of primitive bone shapes; simple
tuft billboards or L-system stubs) or commissioning a small custom pack
is probably the better investment than continuing to trawl Thingiverse.

---

## Skulls

| Name                                                                                                                                           | Author                                                 | URL                                                                                                                        | License (exact quote)                                                                    | Notes                                                                                                                                                                                                                                                                                 |
| ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Gorilla Skull                                                                                                                                  | University of Dundee Museum Collections (@uod_museums) | <https://sketchfab.com/3d-models/gorilla-skull-d948a97bd49841949d29f664c2e6b642>                                           | "CC0 1.0 Universal Public Domain Dedication"                                             | Real museum surface scan (DUNUC 13182), 27cm. Photogrammetry/CT-derived — Dundee's CC0 batch is generally cleaned for reuse but confirm watertightness before print. Good as a "beast skull" for fantasy scatter.                                                                     |
| Lion Skull                                                                                                                                     | University of Dundee Museum Collections                | <https://sketchfab.com/3d-models/lion-skull-b7b59f40f37b4ea99a59a16e17d033f9>                                              | CC0 1.0 Universal                                                                        | 30cm, DUNUC 2021. Same collection, same caveats.                                                                                                                                                                                                                                      |
| Raven Skull                                                                                                                                    | Terrie (@anthroterrie)                                 | <https://www.printables.com/model/1396803-raven-skull>                                                                     | "This work is licensed under a Creative Commons (4.0 International License) Attribution" | **Best all-round candidate.** A print-prepped remix of a Dundee CC0 microCT scan — internal structures removed, holes filled, thin walls thickened specifically for FDM. Requires attribution only (credits panel line). 1,284 likes, 528 collections — well vetted by the community. |
| Brown Bear skull / Sea lion skull / Penguin skull / Chimpanzee skull / Sea otter skull / Right Whale Dolphin skull / Archaeopteryx fossil cast | University of Dundee Museum Collections                | Collection: <https://sketchfab.com/uod_museums/collections/open-access-cc0-public-domain-3b3ffee70cc04e01952dd6803b366074> | CC0 1.0 (collection titled "Open Access – CC0 Public Domain")                            | 10-model collection, listed here as one row since each needs its own manifold/format check before use. Raw scans — expect to need solidify/remesh in Blender before slicing.                                                                                                          |

**Flagged UNSAFE / do not use:**

| Name                                                                  | Where                                                                                       | Why flagged                                                                                                                                                                                                          |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| "Skyrim Dragon Skull" / "Dragon skull from Skyrim" (multiple uploads) | Thingiverse (e.g. thing:1386439, thing:3319714)                                             | Explicitly named after a Bethesda game asset — almost certainly a fan recast of copyrighted in-game geometry. Whatever CC license the uploader slapped on it, they didn't own the underlying design.                 |
| "Proxy Skull Demon (C3V)"                                             | Thingiverse thing:4352649                                                                   | Named as a "proxy" — strongly implies it's a stand-in for a specific commercial miniature line's sculpt. Same recast risk as above.                                                                                  |
| "Skulls" (marble skulls w/ crowns)                                    | MyMiniFactory, Scan the World, <https://www.myminifactory.com/object/3d-print-skulls-48574> | Licensed "BY-NC-SA" — NC and SA both disqualify.                                                                                                                                                                     |
| "High quality skull.stl"                                              | Wikimedia Commons, <https://commons.wikimedia.org/wiki/File:High_quality_skull.stl>         | CC BY-SA 4.0 — SA disqualifies (would drag bundle content under share-alike). Ubiquitous file, keeps getting reposted elsewhere — worth remembering as a trap when it turns up license-stripped on aggregator sites. |

---

## Bones

**This category came up dry for anything actually usable.** Everything
found was either NC/SA-licensed hobbyist work or a paid file whose license
covers personal printing, not redistribution inside a product.

| Name                                     | Author                          | URL                                                                             | License (exact quote)                                                                                                                                                                                                                         | Verdict                                                                                                                                                        |
| ---------------------------------------- | ------------------------------- | ------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Human Pelvis Bone (Anglo-Saxon)          | Three D Scans (threedscans.com) | <https://threedscans.com/lincoln/bone/>                                         | No explicit on-page license statement found; the site is informally known ("free 3d scans... without copyright restrictions" per a Blender Artists thread) as public-domain-ish, but I could not find that written on threedscans.com itself. | **Needs verification** — check threedscans.com/info/ again carefully (my fetch got a stripped page) or email <contact@threedscans.com> before relying on this. |
| "Pile of Bones for D&D Scatter Terrain"  | Lynq                            | <https://www.thingiverse.com/thing:1620123>                                     | "CC BY-NC-SA"                                                                                                                                                                                                                                 | **Unusable** — NC + SA.                                                                                                                                        |
| "Scatter Bones for Basing – Basing Bits" | Voy Forges (@VoyMakesMinis)     | <https://www.myminifactory.com/object/3d-print-scatter-bones-for-basing-366839> | "MyMiniFactory Digital File Store License (Standard)", $3 paid                                                                                                                                                                                | **Unusable for bundling** regardless of price — MMF's standard store license is a print-for-yourself license, not a redistribution grant.                      |

Recommendation: either generate simple bone primitives procedurally
(a femur/rib/skull-fragment as a handful of capsule/lathe shapes — cheap
to make manifold) or commission 10-15 loose bone pieces outright. The
Dundee-style CC0 museum-scan programs occasionally publish full skeletons;
worth a follow-up search specifically for CC0 osteology archives (e.g.
university vet/anatomy departments) rather than hobbyist upload sites.

---

## Rocks / rubble

| Name                                                     | Author        | URL                                                                                                  | License (exact quote)                                                                                                         | Notes                                                                                                                                                                                                                                                          |
| -------------------------------------------------------- | ------------- | ---------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Stylized Nature MegaKit (27 rocks among 110+ models)     | Quaternius    | <https://quaternius.com/packs/stylizednaturemegakit.html> (also on itch.io, OpenGameArt, poly.pizza) | CC0 — "free to use in personal, educational and commercial projects... even for commercial uses and without having to credit" | OBJ/FBX/glTF, stylized low-poly. Not print-ready out of the box — real-time meshes, likely thin-walled/open-bottomed; exactly the kind of raw material our Blender remesh/decimate pass should target, going the _opposite_ direction (thicken, not simplify). |
| Nature Kit (330 assets, incl. rocks/terrain)             | Kenney        | <https://kenney.nl/assets/nature-kit>                                                                | "Creative Commons CC0" (stated on page)                                                                                       | Same low-poly/game-asset caveat as above.                                                                                                                                                                                                                      |
| Pirate Kit (rocks among ships/treasure/trees, 70 assets) | Kenney        | <https://kenney.nl/assets/pirate-kit>                                                                | CC0                                                                                                                           | Bonus: also a source for barrels/crates/treasure chests.                                                                                                                                                                                                       |
| "FREE - 3d Printed Rocks - Scatter Terrain"              | Xykit (Jason) | <https://xykit.com/products/free-3d-printed-rocks-scatter-terrain-stl-file>                          | Page states "©Xykit... See our TERMS OF USE for usage rights" — actual terms not shown on the product page itself             | **Needs verification** — genuinely $0 and no signup, but it's a proprietary site-custom license, not a named CC license. Read the actual Terms of Use page before use; do not assume redistribution rights.                                                    |

---

## Mushrooms

| Name                                                                                                | Author        | URL                                                                                                                | License (exact quote)              | Notes                                                                                                      |
| --------------------------------------------------------------------------------------------------- | ------------- | ------------------------------------------------------------------------------------------------------------------ | ---------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| Mushroom                                                                                            | JeremyWoods   | <https://opengameart.org/content/mushroom-3>                                                                       | "CC0"                              | Low-poly, ships with 3 alternate color textures. Format/poly count not stated on page — check on download. |
| CC0 - 3D Plants (includes mushrooms among trees/grass/ferns/shrubs)                                 | josepharaoh99 | <https://opengameart.org/content/cc0-3d-plants>                                                                    | CC0 — "No attribution is required" | Stylized/low-poly per related-work descriptions.                                                           |
| Stylized Nature MegaKit / Ultimate Nature Pack (mushrooms likely among the "35 plants and flowers") | Quaternius    | <https://quaternius.com/packs/stylizednaturemegakit.html> / <https://quaternius.itch.io/150-lowpoly-nature-models> | CC0                                | Not confirmed mushrooms are literally in the file list — re-check contents on download.                    |

This is the thinnest realistic-quality category, as expected going in.
Nothing sculpted/high-detail and CC0 turned up. If the scatter feature
wants a "good" mushroom (not a stylized game blob), commissioning one or
two is probably worth it — mushrooms are small and cheap to get sculpted.

---

## Plants / leaves / tufts

| Name                               | Author                | URL                                                                     | License (exact quote)                                                                                                                                                                                                                                                        | Notes                                                                    |
| ---------------------------------- | --------------------- | ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| CC0 - 3D Plants                    | josepharaoh99         | <https://opengameart.org/content/cc0-3d-plants>                         | CC0                                                                                                                                                                                                                                                                          | Same pack as above — trees/grass/tropical plants/ferns/shrubs, low-poly. |
| Nature Kit (foliage tag)           | Kenney                | <https://kenney.nl/assets/nature-kit>                                   | CC0                                                                                                                                                                                                                                                                          | 330-asset pack, foliage is one slice of it.                              |
| "Terrain and base grass/reed tuft" | (Cults3D, free model) | <https://cults3d.com/en/3d-model/game/terrain-and-base-grass-reed-tuft> | **Not confirmed** — Cults3D blocked automated fetch during this pass; Cults3D commonly uses a custom "Standard Digital File License" that is personal-print-only, not a redistribution grant. Treat as likely unusable until the license line is read directly off the page. |                                                                          |

Everything purpose-built for wargaming basing that I found — Epic Basing's
grass tuft packs, Vesna Sculpts' "Grass Tufts Basing Bits," Zabavka
Workshop's "Small grass" — is a **paid commercial product with no
redistribution rights**, same as the bones situation. This is the weakest
category overall, exactly as anticipated: the low-poly CC0 game-asset kits
(Kenney/OpenGameArt) are stylized, not the naturalistic static-grass look
a painted base wants. Recommend either accepting the stylized look for a
starter bundle, procedurally generating simple tuft geometry (a handful of
tapered blades/leaves, cheap and controllable), or commissioning.

---

## Gravestones

| Name                                                        | Author | URL                                                                              | License (exact quote)                                  | Notes                                                                                                                                                                                                                                 |
| ----------------------------------------------------------- | ------ | -------------------------------------------------------------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Graveyard Kit (90 assets: graves, coffins, fences, benches) | Kenney | <https://kenney.nl/assets/graveyard-kit>                                         | "Creative Commons CC0"                                 | v5.0, a full remake per Kenney's own changelog. OBJ/FBX/glTF. Best single source for this category — 90 assets covers headstones plus set-dressing (fences, benches).                                                                 |
| CC0 - Tombstone                                             | plaggy | <https://sketchfab.com/3d-models/cc0-tombstone-3990a38c91c5435f84bd2f8c05dfb189> | "CC0 1.0 Universal (CC0 1.0) Public Domain Dedication" | 874 tris / 876 verts, game-ready with PBR textures (4096×4096 maps we won't need). Small and clean — good individual hero piece, but verify manifoldness (game meshes often have open bottoms/non-manifold edges at that poly count). |

**Flagged unusable:** MyMiniFactory "Scan the World" gravestone entries
(e.g. <https://www.myminifactory.com/de/object/3d-print-gravestone-72860>,
licensed "MyMiniFactory Exclusive - Credit - Remix - Noncommercial") — NC
and MMF-platform-exclusive, both disqualifying. Scan the World's default
license across the board skewed NC in everything I sampled — don't assume
"museum-scanned" implies CC0 the way the Dundee collection does.

---

## Barrels / crates / wooden debris

| Name                                                                                | Author        | URL                                                  | License (exact quote)                                                                                                                                                     | Notes                                                                         |
| ----------------------------------------------------------------------------------- | ------------- | ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| Pirate Kit (barrels, crates, ships, treasure — 70 assets)                           | Kenney        | <https://kenney.nl/assets/pirate-kit>                | CC0                                                                                                                                                                       |                                                                               |
| Modular Dungeon Kit (40 assets) / Isometric Dungeon Tiles (crates, barrels, stairs) | Kenney        | <https://kenney.nl/assets/modular-dungeon-kit>       | CC0                                                                                                                                                                       |                                                                               |
| Free Wooden Crates                                                                  | Yughues       | <https://opengameart.org/content/free-wooden-crates> | CC0 — "public domain"                                                                                                                                                     | Weathered/burned/moldy variants — good variety for a "used" battlefield look. |
| Low Poly Storage Pack (35 models: crate, barrel, chest, basket, box, etc.)          | Broken Vector | <https://brokenvector.itch.io/low-poly-storage-pack> | **Ambiguous** — I found a comment thread on the page asking whether it's CC0 or CC-BY with no definitive answer surfaced in my research; the license was not pinned down. | **Needs direct verification** on the itch.io page before use.                 |

**Flagged unusable / unverified-and-suspicious:**

| Name                         | Where                                                                                              | Why                                                                                                                                                                                                                                                                  |
| ---------------------------- | -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| "Barrel and Crate (Terrain)" | Printables, Nygaard Design, <https://www.printables.com/model/853234-barrel-and-crate-40k-terrain> | Licensed "Creative Commons Attribution-NonCommercial 4.0 International (CC BY-NC 4.0)" — NC disqualifies.                                                                                                                                                            |
| "4 Stacked Barrels"          | Printables, Wargame Geeks                                                                          | Not independently verified this pass (fetch didn't reach license text), but Wargame Geeks operates as a commercial terrain studio brand — treat as likely All-Rights-Reserved/NC until checked; don't assume free-and-open just because the listing says "free STL." |

---

## Generic fantasy debris

| Name                      | Author | URL                                          | License (exact quote) | Notes |
| ------------------------- | ------ | -------------------------------------------- | --------------------- | ----- |
| Tiny Dungeon (130 assets) | Kenney | <https://kenney.nl/assets/tiny-dungeon>      | CC0                   |       |
| Mini Dungeon              | Kenney | <https://kenney-assets.itch.io/mini-dungeon> | CC0                   |       |

Covered by the same Kenney dungeon-themed kits as barrels/crates above —
walls, rubble, torches, generic set dressing.

---

## Sites scouted and how they behaved

- **Thingiverse**: license is shown per-thing (CC0/CC-BY/CC-BY-NC/CC-BY-SA
  etc., or "All Rights Reserved"). Lots of volume, but skew heavily
  NC/NC-SA for anything wargaming-scatter-shaped, and recast risk is real
  (see flagged dragon-skull / proxy-demon-skull entries).
- **Printables**: has a `/tag/cc0` browse and per-model license badges
  (CC BY, CC BY-NC, CC0, "Standard Digital File License," etc). The two
  best finds of this whole pass (Raven Skull, Graveyard-kit-adjacent
  searches) trace back through Printables to the Dundee CC0 source.
- **MyMiniFactory**: mixes a proprietary "Digital File Store License"
  (paid, personal-print-only) with actual CC tags. "Scan the World" is
  the big open-culture initiative here but skewed NC in every sample I
  pulled — don't assume it's CC0 by reputation, check each object.
- **Thangs**: search surfaced mostly membership/subscription-gated
  designer content (e.g. "Skull Piles - Scatter Terrain... PRESUPPORTED");
  didn't find genuine CC0 hits worth listing here. Likely worth a second,
  more patient pass with Thangs' own filters rather than web search.
- **Cults3D**: automated fetch was blocked (403) every time during this
  pass, so nothing from Cults3D made it past "possible lead" — its custom
  license system needs a human to actually open the page.
- **CC0 game-asset ecosystem (Kenney, Quaternius, OpenGameArt, itch.io)**:
  this is where the real bulk of usable, unambiguous CC0 material is.
  Tradeoff: these are stylized/low-poly real-time game assets, not
  sculpted photoreal props — expect to need a thicken/remesh/solidify
  pass in Blender before they're print-safe, which lines up with the
  render pipeline we already run (Blender 5.1).
- **CC0 museum-scan programs (University of Dundee Museum Collections on
  Sketchfab, Smithsonian 3D)**: Dundee's "Open Access – CC0 Public Domain"
  collection was the single best skull source found. Smithsonian's
  3d.si.edu blocked automated fetch (CAPTCHA/verification wall) every
  time, so I could not confirm exact per-object license wording there
  this pass — it's well known to publish CC0 for many objects, but that
  needs a human visit to confirm per-model, not a repeat of this list.
- **Three D Scans (threedscans.com)**: no explicit on-page license text
  found in this pass; informally described elsewhere as unrestricted, but
  that needs firming up before we rely on it (see Bones section).

## Next steps before any of this touches the installer

1. Re-open every URL above by hand and screenshot/quote the live license
   text — this document is a scouting pass, not a clearance record.
2. Download candidates into a scratch folder, open each in Blender 5.1,
   check manifoldness (the game-asset-derived ones especially), and only
   then decide which get decimated/remeshed for the bundle.
3. For CC-BY entries (Raven Skull; possibly Low Poly Storage Pack pending
   verification), draft the exact CREDITS.md line and confirm the
   credits-panel UI actually surfaces it — per the license, nothing is
   owed per printed object, but the app-level credit is not optional.
4. Follow up specifically on: a CC0 loose-bone source (try university
   veterinary/anatomy CC0 scan programs), a non-stylized CC0 mushroom,
   and pinning down the Low Poly Storage Pack / Xykit rocks / Cults3D
   grass-tuft licenses one way or the other.
