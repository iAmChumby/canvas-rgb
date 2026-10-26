//! Scratch: prints the effect list the way `canvas-rgb effects list` shows it, from
//! the registry, so the list can be checked without running the daemon.
//!
//! Usage: cargo run --release -p canvas-effects --example effect_list

fn main() {
    for f in canvas_effects::registry() {
        let e = f();
        let s = e.schema();
        let mut needs = String::new();
        if s.needs.screen {
            needs.push_str(" screen");
        }
        if s.needs.audio {
            needs.push_str(" audio");
        }
        println!("  {:<16} {:<18}{needs}", s.id, s.name);
        println!("      {}", s.description);
        for p in &s.properties {
            let kind = match &p.kind {
                canvas_proto::PropKind::Color { default } => format!("color #{:02x?}", default.to_hex()),
                canvas_proto::PropKind::Number { min, max, step, default } => {
                    format!("number {min}..{max} step {step} default {default}")
                }
                canvas_proto::PropKind::Bool { default } => format!("bool default {default}"),
                canvas_proto::PropKind::Choice { options, default } => {
                    format!("choice {options:?} default {default}")
                }
                canvas_proto::PropKind::Hue { default } => format!("hue default {default}"),
            };
            println!("      - {} ({kind})", p.label);
        }
        for preset in &s.presets {
            println!("      preset {}", preset.name);
        }
        println!();
    }
}
