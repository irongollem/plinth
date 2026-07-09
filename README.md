# Plinth

[![GitHub Release](https://img.shields.io/github/v/release/irongollem/plinth?include_prereleases&style=flat-square)](https://github.com/irongollem/plinth/releases)
[![GitHub Issues](https://img.shields.io/github/issues/irongollem/plinth?style=flat-square)](https://github.com/irongollem/plinth/issues)
[![GitHub Stars](https://img.shields.io/github/stars/irongollem/plinth?style=flat-square&cacheSeconds=3600)](https://github.com/irongollem/plinth/stargazers)
[![GitHub Downloads](https://img.shields.io/github/downloads/irongollem/plinth/total?style=flat-square)](https://github.com/irongollem/plinth/releases)
[![Contributions Welcome](https://img.shields.io/badge/contributions-welcome-brightgreen.svg?style=flat-square)](CONTRIBUTING.md)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri-purple?style=flat-square)](https://tauri.app/)

Plinth is an opinionated desktop tool for cataloging, rendering, compressing, and bundling STL files "the right way". It provides an easy-to-use interface for organizing a disk-scale 3D model library and packing releases so creators can distribute them efficiently — and collectors can move their own library between systems without losing curation.

**📖 Documentation: [irongollem.github.io/plinth](https://irongollem.github.io/plinth/)** — installation, guides, and format reference.

## Features

- Compress STL files to reduce file size without losing quality.
- Bundle multiple STL files into a single package.
- User-friendly interface for easy operation.
- Supports batch processing of multiple files.
- Cross-platform support (Windows, macOS, Linux).

## Installation

Download the latest release for your platform from the
[releases page](https://github.com/irongollem/plinth/releases):

- **Windows** — `Plinth_<version>_x64-setup.exe`
- **macOS** — `Plinth_<version>_universal.dmg` (Apple Silicon and Intel)
- **Linux** — `.AppImage`, `.deb`, or `.rpm`

> [!IMPORTANT]
> **Plinth is currently unsigned**, so your operating system will warn you
> the first time you install or run it. Code-signing certificates cost
> hundreds of euros per year, which we've chosen not to spend on a free
> beta — the trade-off is one extra click for you:
>
> - **Windows**: SmartScreen shows "Windows protected your PC" — click
>   **More info**, then **Run anyway**.
> - **macOS 15+**: the app is blocked with "Apple could not verify…" —
>   click **Done**, then open **System Settings → Privacy & Security**,
>   scroll down, and click **Open Anyway**. (On macOS 14 and earlier,
>   right-click the app → **Open** → **Open** is enough.)
> - **Linux**: no warnings; make the AppImage executable first
>   (`chmod +x Plinth_*.AppImage`).
>
> These warnings only mean the build isn't registered with Microsoft or
> Apple — not that anything is wrong with it. Plinth is open source and
> every release is built in public by [GitHub Actions](.github/workflows/release.yml)
> from a tagged commit, so you can audit exactly what you're running.

See the [installation guide](docs/INSTALL.md) for step-by-step
instructions, updating, and troubleshooting.

## Usage

1. Open Plinth.
2. Add STL files you want to compress or bundle.
3. Choose your desired compression settings.
4. Click the "Compress" or "Bundle" button.
5. Save the optimized files to your desired location.

## Our Philosophy

Plinth is an opinionated tool, and we believe in "the right way" to manage and distribute 3D printable files. This means:

- **Efficiency:** Optimizing file sizes without compromising detail, making distribution and storage easier.
- **Organization:** Bundling models, metadata, and previews into a clear, structured format (`.3pk`).
- **Reproducibility:** Ensuring that users have all the information needed to understand and use the models.

(Project maintainers: Please expand this section with more specific details about the design choices and goals of Plinth!)

## Recommended IDE Setup for Development

- [VS Code](https://code.visualstudio.com/) + [Volar](https://marketplace.visualstudio.com/items?itemName=Vue.volar) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Type Support For `.vue` Imports in TS

Since TypeScript cannot handle type information for `.vue` imports, they are shimmed to be a generic Vue component type by default. In most cases this is fine if you don't really care about component prop types outside of templates. However, if you wish to get actual prop types in `.vue` imports (for example to get props validation when using manual `h(...)` calls), you can enable Volar's Take Over mode by following these steps:

1. Run `Extensions: Show Built-in Extensions` from VS Code's command palette, look for `TypeScript and JavaScript Language Features`, then right click and select `Disable (Workspace)`. By default, Take Over mode will enable itself if the default TypeScript extension is disabled.
2. Reload the VS Code window by running `Developer: Reload Window` from the command palette.

You can [learn more about Take Over mode](https://github.com/johnsoncodehk/volar/discussions/471).

## Contributing

Contributions are welcome! Please read the [contributing guidelines](CONTRIBUTING.md) first.

## Development Status & Roadmap

Plinth is under active development! We are constantly working on new features and improvements.
You can follow our progress and see planned features in our [To-Do List](todolist.md).

## License

This project is available under a custom source-available license that allows:

- Free usage for personal, educational, and commercial purposes
- Creating and selling content/bundles made with the software

But prohibits:

- Selling or redistributing the software itself
- Creating derivative works based on the software

See [`LICENSE.md`](LICENSE.md) for the complete license terms and [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) for information about included components.

## The `.3pk` release format

Plinth packs and imports releases in its own modular format: a small
`release.3pk` (metadata, previews, licence, and a checksum for every file)
next to one component archive per model or group. Because every component
carries its own BLAKE3 checksum, importing is verified end-to-end, users
can pick exactly which models to import, and opening a newer version of a
release they already have offers only the components that actually
changed — an update to one model never means re-downloading or
re-importing the whole release.

### Distributing and moving models

Models leave a Plinth catalog for two reasons, and the release builder
serves both: **creators distributing their own work** (select models,
"+ Add to release", fill in the details, pack — customers import the
`.3pk` and get the models _with_ your curation: names, poses, scale,
support status, previews) and **moving your own collection** between
your systems, either by packing releases for a checksum-verified
transfer or simply by copying the library folders and rescanning (the
`model.json` sidecars carry the curation). Plinth moves files, it
doesn't grant rights — distribution features exist for creators
publishing their own models, not for passing along purchased ones.

You may also see `model.plinthpack` files inside your own library —
that's the space-saving "packed at rest" storage, **not** a
distribution format. Don't send those to anyone; they're an internal
detail that only your own catalog (via its `pack.json` sidecar) knows
how to read, and the format may change between app versions. A packed
model needs to be unpacked before it can be staged into a release.

- [Format specification](docs/3PK.md) — the frozen v1 manifest schema,
  deduplication rules, and import semantics.
- [Creator guide](docs/CREATORS.md) — what packing produces, how updates
  reach your users, and practical tips for structuring a release.
- [Catalog internals](docs/CATALOG.md) — how imported releases and
  scanned folders become the searchable library.
