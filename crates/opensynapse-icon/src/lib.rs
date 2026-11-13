//! The OpenSynapse app icon, drawn in code (no image files in the repo, no
//! dependencies). See README.md.
//!
//! Motif: a synapse node with three branches on a dark rounded square. Shapes are
//! signed-distance tests, anti-aliased by 4×4 supersampling. Small sizes get
//! thicker strokes so the motif stays readable at 16 px.
//!
//! Outputs: [`render`] (straight RGBA), [`png`], [`ico`] (PNG entries) and
//! [`windows_res`] (a compiled `.res` with the icon group as resource 1, which
//! MSVC `link.exe` takes as a plain input file; used by the binaries' build scripts).

mod encode;

pub use encode::{ico, png, windows_res};

/// Sizes embedded in the `.ico` / `.res` (Explorer, taskbar, Start menu, Alt-Tab).
pub const ICO_SIZES: [u32; 7] = [16, 20, 24, 32, 48, 64, 256];

/// Background gradient (top → bottom) and accent gradient (top-left → bottom-right).
const BG_TOP: [f32; 3] = [0x1c as f32, 0x26 as f32, 0x33 as f32];
const BG_BOTTOM: [f32; 3] = [0x0d as f32, 0x12 as f32, 0x19 as f32];
const ACCENT_A: [f32; 3] = [0x3d as f32, 0xe8 as f32, 0xc8 as f32];
const ACCENT_B: [f32; 3] = [0x2a as f32, 0x8c as f32, 0xff as f32];

/// Supersamples per axis.
const SS: u32 = 4;

/// Geometry in unit coordinates (0..1, y down).
struct Geometry {
    hub: (f32, f32),
    hub_r: f32,
    ends: [(f32, f32); 3],
    end_r: f32,
    stroke: f32,
}

fn geometry(size: u32) -> Geometry {
    let px = 1.0 / size as f32;
    let hub = (0.5, 0.54);
    let reach = 0.30;
    // Branches up, lower-left and lower-right (120° apart).
    let ends = [-90.0f32, 150.0, 30.0].map(|deg| {
        let a = deg.to_radians();
        (hub.0 + reach * a.cos(), hub.1 + reach * a.sin())
    });
    Geometry {
        hub,
        hub_r: 0.135f32.max(2.2 * px),
        ends,
        end_r: 0.085f32.max(1.6 * px),
        stroke: 0.07f32.max(1.5 * px),
    }
}

fn rounded_square_dist(x: f32, y: f32) -> f32 {
    let (margin, radius) = (0.03, 0.22);
    let half = 0.5 - margin;
    let qx = (x - 0.5).abs() - (half - radius);
    let qy = (y - 0.5).abs() - (half - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

fn segment_dist(x: f32, y: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((x - cx).powi(2) + (y - cy).powi(2)).sqrt()
}

fn in_motif(g: &Geometry, x: f32, y: f32) -> bool {
    let d = |p: (f32, f32)| ((x - p.0).powi(2) + (y - p.1).powi(2)).sqrt();
    d(g.hub) <= g.hub_r
        || g.ends
            .iter()
            .any(|&e| d(e) <= g.end_r || segment_dist(x, y, g.hub, e) <= g.stroke / 2.0)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// The icon as straight (non-premultiplied) RGBA, `size × size`, rows top to bottom.
pub fn render(size: u32) -> Vec<u8> {
    let g = geometry(size);
    let s = size as f32;
    let n = (SS * SS) as f32;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut bg, mut fg) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / s;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / s;
                    if rounded_square_dist(x, y) <= 0.0 {
                        bg += 1;
                        if in_motif(&g, x, y) {
                            fg += 1;
                        }
                    }
                }
            }
            let (cx, cy) = ((px as f32 + 0.5) / s, (py as f32 + 0.5) / s);
            let back = mix(BG_TOP, BG_BOTTOM, cy);
            let accent = mix(ACCENT_A, ACCENT_B, ((cx + cy) / 2.0).clamp(0.0, 1.0));
            let alpha = bg as f32 / n;
            // The motif lies inside the square: blend it over the background.
            let f = if bg == 0 { 0.0 } else { fg as f32 / bg as f32 };
            let rgb = mix(back, accent, f);
            out.extend(rgb.map(|c| c.round().clamp(0.0, 255.0) as u8));
            out.push((alpha * 255.0).round() as u8);
        }
    }
    out
}

/// For a binary's `build.rs`: on Windows/MSVC targets, write [`windows_res`] to
/// `OUT_DIR` and pass it to the linker, so the exe gets the icon (Explorer,
/// taskbar, Start-menu shortcut, which uses the target's icon 0). Other targets:
/// nothing. Reads only the env vars Cargo sets for build scripts.
pub fn embed_windows_icon() {
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") != "windows" || var("CARGO_CFG_TARGET_ENV") != "msvc" {
        return;
    }
    let path = std::path::Path::new(&var("OUT_DIR")).join("opensynapse-icon.res");
    std::fs::write(&path, windows_res()).expect("write opensynapse-icon.res to OUT_DIR");
    // link.exe accepts a compiled .res as an input file.
    println!("cargo:rustc-link-arg-bins={}", path.display());
    println!("cargo:rerun-if-changed=build.rs");
}

#[cfg(test)]
mod tests;
