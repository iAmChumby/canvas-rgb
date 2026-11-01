//! Scores tiles that already exist, with the scorer the bar uses (`thumbnail::score`).
//!
//! cargo run -p canvas-daemon --example score_tiles -- strips <file.png>...   (a frame strip, as the editor's demo serves it)
//! cargo run -p canvas-daemon --example score_tiles -- bodies <file>...       (a raw thumbnail body, as the API serves it)
//!
//! One line per tile: the lit share (lightness at least `LIT_LIGHTNESS`), rows and columns lit,
//! the seam, the mean and the largest step between two frames, and the band of unlit rows.
use canvas_daemon::thumbnail;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((mode, files)) = args.split_first() else {
        eprintln!("usage: score_tiles strips|bodies <file>...");
        std::process::exit(2);
    };
    for file in files {
        let bytes = std::fs::read(file).unwrap_or_else(|e| panic!("cannot read {file}: {e}"));
        let body = match mode.as_str() {
            "strips" => thumbnail::body_from_strip_png(&bytes).unwrap_or_else(|e| panic!("{file}: {e}")),
            "bodies" => bytes,
            other => panic!("unknown mode {other}"),
        };
        let s = thumbnail::score(&body).unwrap_or_else(|| panic!("{file}: not a well formed tile"));
        let name = std::path::Path::new(file).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        println!(
            "{name:<18} lit {:>3.0} %  rows {:>2}/{}  cols {:>2}/{}  seam {:>4.1} %  step {:>4.1} %  max {:>4.1} %  band {}",
            s.lit_avg * 100.0,
            s.rows_lit,
            thumbnail::HEIGHT,
            s.cols_lit,
            thumbnail::WIDTH,
            s.seam * 100.0,
            s.step * 100.0,
            s.max_step * 100.0,
            s.band_rows
        );
    }
}
