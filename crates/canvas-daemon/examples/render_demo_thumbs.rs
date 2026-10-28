//! Renders every built-in effect headlessly, the way `GET /api/v1/effects/{id}/thumbnail` does
//! (same `canvas_daemon::thumbnail::render`, schema defaults, no device touched), and writes each
//! as one PNG frame strip: 64 px wide, the 24 frames stacked top to bottom. The editor's demo
//! backend serves these files, so the demo gallery shows the real effects.
//!
//! Usage: cargo run -p canvas-daemon --example render_demo_thumbs [out_dir]
//! Default out_dir: app/web/public/demo-thumbs

use canvas_proto::EFFECT_IDS;
use std::path::PathBuf;

/// The most one effect's strip may take.
const BUDGET_BYTES: u64 = 60_000;

fn main() {
    let out_dir = PathBuf::from(
        std::env::args().nth(1).unwrap_or_else(|| "app/web/public/demo-thumbs".to_string()),
    );
    std::fs::create_dir_all(&out_dir).expect("create the output folder");
    let mut total = 0usize;
    for id in EFFECT_IDS {
        let mut effect = canvas_effects::create(id).expect("a built-in effect builds");
        let props = effect.schema().defaults();
        let body = canvas_daemon::thumbnail::render(effect.as_mut(), &props);
        let w = u16::from_le_bytes([body[0], body[1]]) as u32;
        let h = u16::from_le_bytes([body[2], body[3]]) as u32;
        let frames = u16::from_le_bytes([body[4], body[5]]) as u32;
        // Drop the low bits of each channel until the strip fits the budget: smooth gradients
        // compress to a fraction of their size, and a 64 x 40 tile shows no banding at this scale.
        let path = out_dir.join(format!("{id}.png"));
        let mut dropped = 3u8;
        loop {
            let mask = 0xFFu8 << dropped;
            let half = 1u8 << (dropped - 1);
            let rgb: Vec<u8> = body[6..].iter().map(|v| v.saturating_add(half) & mask).collect();
            let file = std::fs::File::create(&path).expect("create the png");
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h * frames);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::High);
            enc.set_filter(png::Filter::Paeth);
            enc.write_header().expect("png header").write_image_data(&rgb).expect("png data");
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size <= BUDGET_BYTES || dropped >= 5 {
                break;
            }
            dropped += 1;
        }
        let size = std::fs::metadata(&path).map(|m| m.len() as usize).unwrap_or(0);
        total += size;
        println!("{id:<16} {w}x{h} x {frames} frames  {size:>7} bytes  {} levels  {}", 256u32 >> dropped, path.display());
    }
    println!("total {total} bytes for {} effects", EFFECT_IDS.len());
}
