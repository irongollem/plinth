//! Example: render a printable STL mini in the "resin promo" style by shelling
//! out to headless Blender + render_mini.py. No Blender-Rust bindings needed.
//!
//! Assumes `blender` is on PATH (or set BLENDER_BIN). Adjust paths as needed.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Options that map to render_mini.py flags. Defaults match the locked look.
pub struct RenderOpts {
    pub rotate: [f32; 3], // default [90.0, 0.0, 0.0] — stands up based minis
    pub out: Option<PathBuf>, // default: <first stl>.png next to the model
    pub color: Option<[f32; 3]>, // resin base color
    pub res: Option<u32>,
    pub samples: Option<u32>,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self { rotate: [90.0, 0.0, 0.0], out: None, color: None, res: None, samples: None }
    }
}

/// Render one mini. `parts` are all STL files that make up the mini
/// (e.g. body + base + weapon); they're joined into one object.
pub fn render_mini(parts: &[PathBuf], script: &Path, opts: &RenderOpts) -> std::io::Result<PathBuf> {
    let blender = std::env::var("BLENDER_BIN").unwrap_or_else(|_| "blender".into());

    let mut cmd = Command::new(blender);
    cmd.arg("-b")                      // background / headless
        .arg("-P").arg(script)         // our render script
        .arg("--");                    // everything after this goes to the script

    for p in parts {
        cmd.arg(p);
    }
    cmd.arg("--rotate")
        .arg(format!("{},{},{}", opts.rotate[0], opts.rotate[1], opts.rotate[2]));
    if let Some(c) = opts.color {
        cmd.arg("--color").arg(format!("{},{},{}", c[0], c[1], c[2]));
    }
    if let Some(r) = opts.res { cmd.arg("--res").arg(r.to_string()); }
    if let Some(s) = opts.samples { cmd.arg("--samples").arg(s.to_string()); }
    let out = opts.out.clone().unwrap_or_else(|| parts[0].with_extension("png"));
    cmd.arg("--out").arg(&out);

    let status = cmd.status()?;
    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("blender exited with {status}"),
        ));
    }
    Ok(out)
}

fn main() -> std::io::Result<()> {
    let script = PathBuf::from("render_mini.py");

    // Single-part mini:
    // let parts = vec![PathBuf::from(r"Z:\...\GiantNewt_v02.stl")];

    // Multi-part mini (body + base):
    let parts = vec![
        PathBuf::from(r"Z:\Dragon Trappers Lodge\2026-06 Critterfolk of Craggy Bog\giant newt\Unsupported\GiantNewt_v02.stl"),
        PathBuf::from(r"Z:\Dragon Trappers Lodge\2026-06 Critterfolk of Craggy Bog\giant newt\Unsupported\GiantNewt_Base_v02.stl"),
    ];

    let png = render_mini(&parts, &script, &RenderOpts::default())?;
    println!("rendered: {}", png.display());
    Ok(())
}
