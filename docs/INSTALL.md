# Installing Plinth

Plinth is a desktop app for Windows, macOS, and Linux. Every release is
built by public GitHub Actions from a tagged commit and published on the
[releases page](https://github.com/irongollem/stl-pack/releases) — the
repository is named `stl-pack` for historical reasons; the product it
builds is Plinth.

Plinth is currently **beta software** (versions look like
`0.1.0-beta.2`). Expect rough edges, and expect updates.

One thing to know up front: the builds are **not code-signed**. Signing
certificates cost real money every year (the Apple Developer Program and
a Windows code-signing certificate together run roughly $220/year), and
for a free, open-source app that cost is deliberately skipped for now.
The practical consequence is that Windows and macOS will show a scary
warning the first time you run the app. Those warnings mean "this
publisher hasn't paid to be identified", not "this app is malicious" —
but you shouldn't take that on faith. You can verify it yourself: the
[source code](https://github.com/irongollem/stl-pack) is public, and
each release is produced by a public CI run from a tagged commit, so
what you download is what the code says. The sections below walk
through each platform's warning step by step.

## Windows

Download **`Plinth_<version>_x64-setup.exe`** from the
[releases page](https://github.com/irongollem/stl-pack/releases) — the
NSIS installer. (Pre-release builds ship the `.exe` only; an `.msi` for
deployment tooling may return for stable releases.)

### If your browser flags the download

Edge and Chrome sometimes hold back unsigned installers before you even
get to run them:

- **Edge**: in the downloads popup, hover the file, click the `…` menu
  → **Keep** → **Show more** → **Keep anyway**.
- **Chrome**: in the downloads bar or panel, choose **Keep** (you may
  need to expand the entry to find it).

### Running the installer past SmartScreen

Double-clicking the installer shows a blue dialog titled **"Windows
protected your PC"**. To proceed:

1. Click **More info** (small link under the message text).
2. Click **Run anyway**.

The installer then runs normally. SmartScreen's reputation is tracked
**per file**, so a brand-new release binary always starts with none —
this warning will reappear with every new version until enough people
have run that exact file. That is expected, not a regression.

## macOS

Download **`Plinth_<version>_universal.dmg`** — a single universal
build that runs natively on both Apple Silicon and Intel Macs. Open the
`.dmg` and drag **Plinth** into your **Applications** folder.

The app is ad-hoc signed but **not notarized** by Apple, so Gatekeeper
objects on first launch. What you do next depends on your macOS
version.

### macOS 15 (Sequoia) and later

The old right-click trick no longer works here. On first launch you get
**"Apple could not verify 'Plinth' is free of malware…"** with no way
to open it from that dialog:

1. Click **Done** to dismiss the dialog.
2. Open **System Settings → Privacy & Security**.
3. Scroll down to the Security section — you'll see a line saying
   Plinth was blocked, with an **Open Anyway** button.
4. Click **Open Anyway**, then confirm in the dialog that follows
   (macOS asks for your password or Touch ID).

This is needed once per install; after that Plinth opens normally.

### macOS 14 and earlier

1. In **Applications**, **right-click** (or Control-click) Plinth and
   choose **Open**.
2. In the warning dialog, click **Open**.

Subsequent launches work with a normal double-click.

### Terminal fallback (advanced)

macOS attaches a **quarantine** attribute to files downloaded from the
internet; it's that flag that triggers the Gatekeeper check. If you're
comfortable in a terminal, you can clear it directly:

```sh
xattr -cr /Applications/Plinth.app
```

Only do this for software you've decided to trust — it removes the
check rather than passing it.

## Linux

No signing ceremony here; pick the format that fits your distro from
the [releases page](https://github.com/irongollem/stl-pack/releases).

### AppImage (any distro)

```sh
chmod +x Plinth_<version>_amd64.AppImage
./Plinth_<version>_amd64.AppImage
```

AppImages need `libfuse2` on some distros — notably Ubuntu 22.04 and
later, where it's no longer preinstalled:

```sh
sudo apt install libfuse2
```

### Debian / Ubuntu (`.deb`)

```sh
sudo apt install ./Plinth_<version>_amd64.deb
```

### Fedora / openSUSE (`.rpm`)

Install with `dnf` (Fedora) or `zypper` (openSUSE), pointing at the
downloaded file.

## First run

The installers register Plinth as the handler for **`.3pk`** and
**`.3dpak`** files — the 3D model package format Plinth opens (see the
[`.3pk` spec](3PK.md)). Once installed, double-clicking one of those
files opens it in Plinth.

## Updating

Updates are currently **manual**: download the new installer from the
releases page and run it over the existing installation (on macOS,
drag the new app over the old one in Applications). Your settings and
catalog are preserved — they live in the app data directory, not the
install directory, so replacing the app doesn't touch them.

Because the builds are unsigned, the OS warnings described above will
**reappear on every update** — each new release is a new, unrecognized
binary. Walk through the same steps as before.

## Uninstalling

- **Windows**: Settings → Apps → Installed apps → Plinth → Uninstall.
- **macOS**: drag `Plinth.app` from Applications to the Trash.
- **Linux**: `sudo apt remove plinth` / `sudo dnf remove plinth` for
  the packaged installs; for the AppImage, just delete the file.

Uninstalling removes the app, not your library or catalog data.

## FAQ

### Why is the app unsigned?

Code signing costs roughly $220/year across Apple and Windows
certificates. Plinth is a free indie project, and that money currently
buys nothing except suppressing a dialog. This may change later — if
the project grows, signing is the obvious first spend.

### Is it safe?

You don't have to trust a claim: the code is
[open source](https://github.com/irongollem/stl-pack), and every
release binary is built by a public GitHub Actions workflow from a
tagged commit. Anyone can read the code, inspect the CI run that
produced a release, or build the same thing themselves.

### My antivirus flagged the download

False positives are common for unsigned Rust binaries — heuristic
scanners are suspicious of anything new and unrecognized. If it
happens, report the false positive to your antivirus vendor (most have
a submission form), or build from source and sidestep the question
entirely.

## Building from source

You need stable [Rust](https://rustup.rs) and [Bun](https://bun.sh):

```sh
bun install
bun run tauri build
```

The bundles land in `src-tauri/target/release/bundle/`. For a full
development setup, see
[CONTRIBUTING.md](https://github.com/irongollem/stl-pack/blob/main/CONTRIBUTING.md).
