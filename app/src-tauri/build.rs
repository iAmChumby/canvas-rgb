//! `tauri::generate_context!()` needs `../web/out` to exist at compile time. The editor's
//! real static export overwrites this placeholder; it is a gitignored build output.

use std::path::Path;

fn main() {
    let index = Path::new("../web/out/index.html");
    if !index.exists() {
        if let Some(dir) = index.parent() {
            std::fs::create_dir_all(dir).expect("create app/web/out for the Tauri build");
        }
        std::fs::write(
            index,
            "<!doctype html><title>canvas-rgb</title><p>The editor has not been built yet (app/web/out is empty).</p>\n",
        )
        .expect("write the placeholder app/web/out/index.html");
    }
    // The exe's icon resource (taskbar, Start menu, Installed apps) is embedded here. Without
    // this, a changed icon file did not re-run the script and the old icon shipped.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build();
}
