//! Hardware probe for the ROYUAN MonsGeek keyboard: direct mode, then red, green, blue
//! (one second each), then black. TOUCHES REAL HARDWARE: run it only with someone watching.

use canvas_adapters::royuan::RoyuanAdapter;
use canvas_proto::color::Rgb8;
use canvas_proto::device::DeviceId;
use canvas_proto::traits::DeviceAdapter;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let mut a = RoyuanAdapter::new();
    let found = match a.discover().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("discover failed: {e}");
            return;
        }
    };
    let Some(dev) = found.first() else {
        eprintln!("no ROYUAN keyboard vendor interface found (3151:4015, if 2, 0xFFFF/0x0002)");
        return;
    };
    println!("found {} ({})", dev.name, dev.id);
    let id: DeviceId = dev.id.clone();
    println!("connecting and sending direct mode");
    if let Err(e) = a.connect().await {
        eprintln!("connect failed: {e}");
        return;
    }
    // Each colour is held for `hold` seconds (first argument, default 3), written at 30 fps
    // so the adapter's 1 s keepalive re-sends it during the hold.
    let hold: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let start = std::time::Instant::now();
    for (name, c) in [
        ("red", Rgb8::new(255, 0, 0)),
        ("green", Rgb8::new(0, 255, 0)),
        ("blue", Rgb8::new(0, 0, 255)),
        ("white", Rgb8::new(255, 255, 255)),
        ("black", Rgb8::BLACK),
    ] {
        println!("{:>5.1}s {name}", start.elapsed().as_secs_f32());
        let until = std::time::Instant::now() + Duration::from_secs(hold);
        while std::time::Instant::now() < until {
            if let Err(e) = a.write_frame(&id, &[c]).await {
                eprintln!("write failed: {e}");
                return;
            }
            tokio::time::sleep(Duration::from_millis(33)).await;
        }
    }
    let _ = a.disconnect().await;
    println!("done");
}
