//! The `cause:regression` class guard for the daemon's own effects (#407).
//!
//! An effect that holds another effect has to answer how a strip reads it: the strip reads the
//! wrapper's picture, and the wrapper's picture is the inner effect's picture. A wrapper that
//! answers the cell while the effect inside answers the column makes a strip read dark, which
//! is what #407 was.
//!
//! This module holds the guard: one test over every effect the daemon implements, and a scan
//! over the daemon's own source that fails with `file:line` when an effect appears that the
//! test does not cover.

#[cfg(test)]
mod tests {
    use crate::pattern_effect::PatternEffect;
    use crate::stack::{AreaMap, AreaMaps, LeafBuilder, LiveMap, StackEffect};
    use crate::user_effects::TileHtml;
    use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
    use canvas_proto::pattern::{BuiltOn, Pattern};
    use canvas_proto::props::Props;
    use canvas_proto::scene::Arrangement;
    use canvas_proto::stack::{LayerArea, Stack, StackLayer};
    use canvas_proto::traits::{Effect, StripRead};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// The daemon's own effects, and the rule each one follows.
    ///
    /// The scan below says that every effect in the daemon's production source is on this list,
    /// so an effect that is added has to come here and be driven by this test.
    const GUARDED: &[(&str, &str)] = &[
        (
            "StackEffect",
            "the every-layer rule: the stack answers the column when every layer that paints \
             answers it, and the cell otherwise",
        ),
        ("TileHtml", "the read of the effect it holds"),
        ("PatternEffect", "holds no effect: a pattern has no picture"),
    ];

    /// #407: every effect the daemon implements answers how a strip reads it.
    ///
    /// - the **stack** follows the every-layer rule: a stack of flames answers the column, a
    ///   stack that mixes a flame with another effect answers the cell;
    /// - the **HTML tile** forwards the read of the effect it holds;
    /// - the **pattern** holds no effect: it draws nothing, and the engine reads its colours
    ///   per LED.
    ///
    /// The scan runs first: an effect that is not under this guard fails here, with its file
    /// and its line.
    #[test]
    fn every_effect_the_daemon_implements_answers_how_a_strip_reads_it() {
        // The scan first: an effect that is not under this guard fails here, with its file and
        // its line. A guarded effect that is gone fails too, so the list cannot go stale.
        let found = scan_effects();
        let findings: Vec<String> = found
            .iter()
            .filter(|(_, _, type_name)| !GUARDED.iter().any(|(name, _)| name == type_name))
            .map(|(rel, line, type_name)| {
                format!(
                    "{rel}:{line}: {type_name} is not under the strip-read guard: add it to \
                     GUARDED and drive it in this test"
                )
            })
            .collect();
        assert!(
            findings.is_empty(),
            "the strip-read guard found effects it does not drive:\n{}",
            findings.join("\n")
        );
        let stale: Vec<String> = GUARDED
            .iter()
            .filter(|(name, _)| !found.iter().any(|(_, _, type_name)| type_name == name))
            .map(|(name, rule)| format!("{name} is guarded but the daemon does not implement it: {rule}"))
            .collect();
        assert!(
            stale.is_empty(),
            "the strip-read guard names effects that are gone:\n{}",
            stale.join("\n")
        );

        // The stack: the every-layer rule.
        let flames = Stack {
            id: "stack:Guard".into(),
            layers: vec![layer("l1", "fire"), layer("l2", "fire")],
        };
        let fire = StackEffect::new(&flames, leaves(), live(&flames), areas());
        assert_eq!(
            fire.strip_read(),
            StripRead::Column,
            "a stack of flames answers the column"
        );
        let mixed = Stack {
            id: "stack:Guard".into(),
            layers: vec![layer("l1", "fire"), layer("l2", "rain")],
        };
        let leaves: LeafBuilder = Arc::new(|id: &str| match id {
            "fire" => Ok(Box::new(canvas_effects::fire::Fire::with_seed(372)) as Box<dyn Effect>),
            "rain" => Ok(Box::new(canvas_effects::rain::Rain::with_seed(372)) as Box<dyn Effect>),
            other => Err(format!("no effect with the id {other}")),
        });
        let mixed = StackEffect::new(&mixed, leaves, live(&mixed), areas());
        assert_eq!(
            mixed.strip_read(),
            StripRead::Cell,
            "a stack that mixes a flame with another effect answers the cell"
        );

        // The HTML tile: the read of the effect it holds.
        let source = r##"<head><title>Guard</title></head>
<body><canvas id="stage" width="32" height="32"></canvas></body>
<script>
  var ctx = document.getElementById("stage").getContext("2d");
  function draw() {
    ctx.fillStyle = "#204060";
    ctx.fillRect(0, 0, 32, 32);
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;
        let inner = crate::user_effects::html_effect("guard", source).expect("the effect builds");
        let inner_read = inner.strip_read();
        let tile = TileHtml(inner.tile_mode());
        assert_eq!(
            tile.strip_read(),
            inner_read,
            "the tile forwards the read of the effect it holds"
        );

        // The pattern: it holds no effect and draws nothing.
        let pattern = PatternEffect::new(pattern());
        assert_eq!(
            pattern.strip_read(),
            StripRead::Cell,
            "a pattern draws nothing, so it reads the cell"
        );
    }

    /// A stack layer.
    fn layer(id: &str, effect: &str) -> StackLayer {
        StackLayer {
            id: id.to_string(),
            name: format!("Layer {id}"),
            effect: effect.to_string(),
            props: Props::new(),
            area: LayerArea::Canvas,
            blend: canvas_proto::stack::BlendMode::Normal,
            opacity: 100,
            visible: true,
            mask: None,
        }
    }

    /// A stack whose layers are all flames.
    fn leaves() -> LeafBuilder {
        Arc::new(|id: &str| match id {
            "fire" => Ok(Box::new(canvas_effects::fire::Fire::with_seed(372)) as Box<dyn Effect>),
            other => Err(format!("no effect with the id {other}")),
        })
    }

    /// The live map a stack reads.
    fn live(stack: &Stack) -> Arc<std::sync::RwLock<LiveMap>> {
        Arc::new(std::sync::RwLock::new(LiveMap::from_stack(stack, 1)))
    }

    /// The area book a stack reads: one group with one strip.
    fn areas() -> Arc<std::sync::RwLock<AreaMaps>> {
        let devices = vec![device()];
        let (lights, plan) = crate::stack::zone_group(&devices);
        let map = AreaMap::build_for_group(
            &plan,
            &lights,
            canvas_proto::layout::SamplingMode::AreaAverage,
            1,
        );
        Arc::new(std::sync::RwLock::new(AreaMaps {
            version: 1,
            groups: [("g".to_string(), map)].into_iter().collect(),
        }))
    }

    /// A device with one strip of eight LEDs.
    fn device() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId("d".into()),
            name: "d".into(),
            vendor: "fake".into(),
            kind: DeviceKind::Other,
            adapter: "fake".into(),
            adapter_index: 0,
            serial: String::new(),
            location: String::new(),
            zones: vec![ZoneInfo {
                index: 0,
                name: "z".into(),
                kind: ZoneKind::Linear,
                led_count: 8,
                min_leds: 8,
                max_leds: 8,
                resizable: false,
            }],
            connected: true,
        }
    }

    /// A saved pattern.
    fn pattern() -> Pattern {
        Pattern {
            id: "pattern:Guard".into(),
            built_on: BuiltOn {
                led_count: 12,
                arrangement: Arrangement::Ring,
                grid: None,
            },
            frames: Vec::new(),
            motion: canvas_proto::pattern::Motion::none(),
            playback: None,
            extra: serde_json::Map::new(),
        }
    }

    // ------------------------------------------------------------ the scan

    /// The daemon's own source directory.
    fn src_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    /// Every Rust file of the daemon's own source.
    fn source_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut dirs = vec![dir.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
    }

    /// Whether a line is an attribute that makes the next item test-only: `#[cfg(test)]` or
    /// `#[cfg(all(test, ...))]`. `#[cfg(not(test))]` and `#[cfg(any(test, ...))]` keep the item
    /// in a release build, so they are production code.
    fn is_test_cfg(line: &str) -> bool {
        let line = line.trim();
        line == "#[cfg(test)]" || (line.starts_with("#[cfg(all(test") && line.ends_with(")]"))
    }

    /// The leading whitespace of a line.
    fn indent(line: &str) -> &str {
        &line[..line.len() - line.trim_start().len()]
    }

    /// The first line of the item an attribute at line `at` applies to (after further
    /// attributes).
    fn item_after(lines: &[&str], at: usize) -> usize {
        let mut item = at + 1;
        while item < lines.len() && lines[item].trim().starts_with("#[") {
            item += 1;
        }
        item
    }

    /// The lines of a file that are test code: everything from a test-only attribute to the end
    /// of the item it applies to.
    fn test_lines(lines: &[&str]) -> Vec<bool> {
        let mut skip = vec![false; lines.len()];
        let mut i = 0;
        while i < lines.len() {
            if !is_test_cfg(lines[i]) {
                i += 1;
                continue;
            }
            let item = item_after(lines, i);
            let Some(first) = lines.get(item) else {
                skip[i] = true;
                break;
            };
            let end = if first.trim_end().ends_with(';') || first.trim_end().ends_with('}') {
                // A declaration, or an item that closes on its own line.
                item
            } else {
                // An item with a body: it ends at the line that closes it at the item's indent.
                let margin = indent(first);
                (item + 1..lines.len())
                    .find(|&k| lines[k].trim() == "}" && indent(lines[k]) == margin)
                    .unwrap_or(lines.len() - 1)
            };
            for line in skip.iter_mut().take(end + 1).skip(i) {
                *line = true;
            }
            i = end + 1;
        }
        skip
    }

    /// The type of an `impl ... for ... {` line, when it implements the `Effect` trait.
    fn effect_impl(line: &str) -> Option<String> {
        let rest = line.trim().strip_prefix("impl ")?;
        let (trait_path, type_path) = rest.split_once(" for ")?;
        let trait_name = trait_path.trim().rsplit("::").next()?;
        if trait_name != "Effect" {
            return None;
        }
        let name = type_path.trim().trim_end_matches('{').trim();
        let name = name.split('<').next()?.trim();
        Some(name.rsplit("::").next()?.to_string())
    }

    /// Everything the scan found: the effects in the daemon's production source, with the file
    /// and the line they are on.
    fn scan_effects() -> Vec<(String, usize, String)> {
        let dir = src_dir();
        let files = source_files(&dir);
        let mut found: Vec<(String, usize, String)> = Vec::new();
        for path in files {
            let text = fs::read_to_string(&path).expect("a source file reads");
            let lines: Vec<&str> = text.lines().collect();
            let skip = test_lines(&lines);
            for (i, line) in lines.iter().enumerate() {
                if skip[i] {
                    continue;
                }
                if let Some(type_name) = effect_impl(line) {
                    let rel = path
                        .strip_prefix(&dir)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    found.push((rel, i + 1, type_name));
                }
            }
        }
        assert!(!found.is_empty(), "the scan found no effect at all: the roots moved?");
        found
    }
}
