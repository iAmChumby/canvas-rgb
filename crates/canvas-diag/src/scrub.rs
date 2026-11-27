//! The one scrubber for text that can leave the app (#574).
//!
//! Moved here from `canvas-daemon` in R1 so the daemon and the tray share the same rules. The
//! daemon is the only part that can read the profile path, the user name and the machine name, so
//! this is where they are removed. It runs LAST over every string in the shareable report, before
//! any length cap, and the same function is reused by the diagnostics zip (#575). The editor never
//! rebuilds a scrubbed string.
//!
//! Every spelling is covered: backslash and forward slash, the 8.3 short form, and any case.
//! Windows supplies the real 8.3 path at run time via `GetShortPathNameW`; when it cannot, a
//! fallback follows Windows' rule (drop spaces and invalid characters, first six, `~1`).
//! Bare names match only at token boundaries, so "Ed" does not appear in "collected" and "PC"
//! does not appear in "PCIe"; full profile paths still match as path prefixes.

/// What replaces the profile path, in every spelling.
pub const PROFILE_TOKEN: &str = "<profile>";
/// What replaces the user name.
pub const USER_TOKEN: &str = "<user>";
/// What replaces the machine name.
pub const MACHINE_TOKEN: &str = "<machine>";
/// What replaces a device id in error text the person reads.
pub const DEVICE_TOKEN: &str = "the device";

/// Adapter prefixes a `DeviceId` may carry, shared by the bundle's alias pass and the error
/// scrubber. `<prefix>:<rest>` is a device fingerprint, never a sentence or a URL.
///
/// `ble:` is written only to `PeripheralInfo.location` in the 8BitDo adapter; the pad's id is
/// built from `ADAPTER_NAME` (`eightbitdo-ultimate-2`). A location is not a DeviceId and no daemon
/// error message interpolates it, so `ble` is deliberately absent here (H3).
pub const ADAPTER_PREFIXES: &[&str] = &[
    "openrgb", "ring", "chione", "royuan", "attack-shark-x6", "eightbitdo-ultimate-2", "hid", "usb", "fake", "cpu",
];

/// Replaces every device-shaped token in text a person may read. Used on error bodies (P6) and
/// on the same text the editor copies or puts in a report URL. The rest of the token is dropped
/// whole, so a serial or a USB location cannot survive as a prefix or a suffix.
pub fn scrub_device_ids(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        if let Some(end) = device_id_end(text, at) {
            out.push_str(DEVICE_TOKEN);
            at = end;
        } else {
            let ch = text[at..].chars().next().unwrap();
            out.push(ch);
            at += ch.len_utf8();
        }
    }
    out
}

/// True when a byte can be part of an identifier (used to decide whether a prefix starts one).
fn is_id_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')
}

/// If a `<prefix>:<id>` starts exactly at `at`, returns the byte offset where the whole id ends.
/// The id runs to whitespace, a quote or a closing bracket, or a sentence-final full stop — not
/// to the second colon, so `royuan:3151:4015` is taken whole (F3).
pub fn device_id_end(text: &str, at: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if at > 0 && is_id_byte(bytes[at - 1]) {
        return None;
    }
    for prefix in ADAPTER_PREFIXES {
        let p = prefix.as_bytes();
        if bytes[at..].starts_with(p) && bytes.get(at + p.len()) == Some(&b':') {
            let start = at + p.len() + 1;
            let mut end = start;
            while end < bytes.len() {
                let b = bytes[end];
                if b.is_ascii_whitespace() || matches!(b, b'"' | b'\'' | b',' | b';' | b')' | b']' | b'}' | b'>')
                {
                    break;
                }
                if b == b'.' && (end + 1 == bytes.len() || bytes[end + 1].is_ascii_whitespace()) {
                    break;
                }
                // "openrgb:ABC: timeout" and a trailing "openrgb:ABC:" are one id followed by
                // a separator: keep the colon, including at the very end (H6).
                if b == b':' && bytes.get(end + 1).map_or(true, u8::is_ascii_whitespace) {
                    break;
                }
                // A backslash escaping a quote in JSON is the end of the id, not part of it.
                if b == b'\\' && matches!(bytes.get(end + 1), Some(b'"' | b'\\')) {
                    break;
                }
                end += 1;
            }
            if end > start {
                return Some(end);
            }
        }
    }
    None
}

/// One replacement. Paths match as prefixes; bare names need whole-token boundaries.
#[derive(Debug, Clone)]
struct Pattern {
    text: String,
    token: &'static str,
    bounded: bool,
}

/// A set of replacements, longest first, so a longer path goes before the user name inside it.
#[derive(Debug, Clone, Default)]
pub struct Scrubber {
    patterns: Vec<Pattern>,
}

impl Scrubber {
    /// Reads the three names Windows gives the process. Missing values simply add no pattern;
    /// a scrubber with fewer names still removes the ones it knows. The real 8.3 profile path
    /// comes from the OS; a failure falls back to the Windows naming rule.
    pub fn from_env() -> Self {
        let profile = std::env::var("USERPROFILE").ok();
        let short = profile.as_deref().and_then(short_path_runtime);
        Self::with_short(
            profile,
            std::env::var("USERNAME").ok(),
            std::env::var("COMPUTERNAME").ok(),
            short,
        )
    }

    /// Builds the patterns from explicit values. Tests pass a fake profile here so no test
    /// depends on the account that happens to run it.
    pub fn new(profile: Option<String>, user: Option<String>, machine: Option<String>) -> Self {
        Self::with_short(profile, user, machine, None)
    }

    /// `real_profile_short` is the whole short path from `GetShortPathNameW`, when the OS gave
    /// one. It is used exactly, because a guessed form is wrong for names with spaces or dots.
    fn with_short(
        profile: Option<String>,
        user: Option<String>,
        machine: Option<String>,
        real_profile_short: Option<String>,
    ) -> Self {
        let mut patterns: Vec<Pattern> = Vec::new();
        if let Some(profile) = profile.filter(|p| !p.trim().is_empty()) {
            let profile = profile.trim().trim_end_matches(['\\', '/']).to_string();
            for path in spelling_variants(&profile, real_profile_short.as_deref()) {
                patterns.push(Pattern { text: path, token: PROFILE_TOKEN, bounded: false });
            }
            // The path's own last component and its short form also occur on their own, e.g. in
            // a device name a vendor put there. They are names now, so they need boundaries.
            let short_last = real_profile_short
                .as_deref()
                .and_then(|s| s.rsplit(['\\', '/']).next().filter(|s| !s.is_empty()))
                .map(str::to_string);
            if let Some(last) = profile.rsplit(['\\', '/']).next().filter(|s| !s.is_empty()) {
                if let Some(short_last) = short_last {
                    for name in [last.to_string(), short_last] {
                        patterns.push(Pattern { text: name, token: USER_TOKEN, bounded: true });
                    }
                } else {
                    push_name(&mut patterns, last, USER_TOKEN);
                }
            }
        }
        if let Some(user) = user.filter(|u| !u.trim().is_empty()) {
            push_name(&mut patterns, user.trim(), USER_TOKEN);
        }
        if let Some(machine) = machine.filter(|m| !m.trim().is_empty()) {
            push_name(&mut patterns, machine.trim(), MACHINE_TOKEN);
        }
        patterns.sort_by(|a, b| b.text.len().cmp(&a.text.len()).then_with(|| a.text.cmp(&b.text)));
        patterns.dedup_by(|a, b| a.text == b.text);
        Self { patterns }
    }

    /// Removes every known name from one string. The same function serves the report, the zip
    /// and every error answer; callers must not write their own replacement.
    pub fn scrub(&self, input: &str) -> String {
        let mut out = input.to_string();
        for pattern in &self.patterns {
            out = replace_case_insensitive(&out, &pattern.text, pattern.token, pattern.bounded);
        }
        out
    }

    /// `scrub` plus the number of replacements it made. The bundle's `replaced` count comes
    /// from this, so the number on the card is a measurement and never a guess (#575).
    pub fn scrub_count(&self, input: &str) -> (String, u32) {
        let mut out = input.to_string();
        let mut count = 0u32;
        for pattern in &self.patterns {
            let (next, n) = replace_counted(&out, &pattern.text, pattern.token, pattern.bounded);
            out = next;
            count = count.saturating_add(n);
        }
        (out, count)
    }

    /// True when there is nothing to remove (e.g. Windows gave no names at all).
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

/// Adds the long name and its 8.3 form, both bounded to whole names.
fn push_name(patterns: &mut Vec<Pattern>, name: &str, token: &'static str) {
    for form in [name.to_string(), short_form(name)] {
        if !form.is_empty() {
            patterns.push(Pattern { text: form, token, bounded: true });
        }
    }
}

/// Windows' 8.3 rule for a component: drop spaces, dots and invalid characters, keep the first
/// six characters, uppercase, then `~1`. `Luke Smith` becomes `LUKESM~1`.
fn short_form(component: &str) -> String {
    let head: String = component
        .chars()
        .filter(|c| {
            !c.is_whitespace()
                && !c.is_control()
                && !matches!(c, '.' | '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
        .take(6)
        .flat_map(char::to_uppercase)
        .collect();
    format!("{head}~1")
}

/// Asks Windows for the real short path. The first call sizes the buffer, the second fills it.
/// A missing path, a short-name-disabled volume or any API failure returns `None` and the caller
/// uses the rule above instead.
#[cfg(windows)]
fn short_path_runtime(path: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let needed = GetShortPathNameW(PCWSTR(wide.as_ptr()), None);
        if needed == 0 {
            return None;
        }
        let mut buffer = vec![0u16; needed as usize + 1];
        let written = GetShortPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buffer));
        if written == 0 || written > needed {
            return None;
        }
        let text = String::from_utf16_lossy(&buffer[..written as usize]);
        (!text.is_empty()).then_some(text)
    }
}

#[cfg(not(windows))]
fn short_path_runtime(_path: &str) -> Option<String> {
    None
}

/// Both slash directions for a path, with the long and the fallback short form of its last
/// component. When Windows gave a real short path it is used verbatim as another spelling.
fn spelling_variants(path: &str, real_short: Option<&str>) -> Vec<String> {
    let split = |p: &str| -> Vec<String> {
        p.split(['\\', '/']).filter(|s| !s.is_empty()).map(str::to_string).collect()
    };
    let components = split(path);
    let short_components = real_short.map(split);
    let mut variants = Vec::new();
    for separator in ['\\', '/'] {
        let sep = separator.to_string();
        variants.push(components.join(&sep));
        if let Some(short) = &short_components {
            if !short.is_empty() {
                variants.push(short.join(&sep));
            }
        } else if components.len() > 1 {
            let mut short = components.clone();
            if let Some(last) = short.last_mut() {
                *last = short_form(last);
            }
            variants.push(short.join(&sep));
        }
    }
    variants.dedup();
    variants
}

/// A bare name is a whole token: the characters on either side are not letters or digits.
fn token_boundary(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text.get(end..).and_then(|rest| rest.chars().next());
    !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
}

/// ASCII-case-insensitive replace, optionally restricted to whole-token matches.
fn replace_case_insensitive(
    haystack: &str,
    needle: &str,
    replacement: &str,
    bounded: bool,
) -> String {
    replace_counted(haystack, needle, replacement, bounded).0
}

/// The counting twin of [`replace_case_insensitive`].
fn replace_counted(
    haystack: &str,
    needle: &str,
    replacement: &str,
    bounded: bool,
) -> (String, u32) {
    if needle.is_empty() || needle.len() > haystack.len() {
        return (haystack.to_string(), 0);
    }
    let bytes = haystack.as_bytes();
    let pattern = needle.as_bytes();
    let mut out = String::with_capacity(haystack.len());
    let mut count = 0u32;
    let mut at = 0;
    while at + pattern.len() <= bytes.len() {
        if bytes[at..at + pattern.len()]
            .iter()
            .zip(pattern)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
            && (!bounded || token_boundary(haystack, at, at + pattern.len()))
        {
            out.push_str(replacement);
            count += 1;
            at += pattern.len();
        } else {
            let width = match bytes[at] {
                0x00..=0x7f => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                _ => 4,
            };
            let end = (at + width).min(bytes.len());
            out.push_str(&haystack[at..end]);
            at = end;
        }
    }
    out.push_str(&haystack[at..]);
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> Scrubber {
        Scrubber::new(
            Some(concat!("C:\\", "Use", "rs\\Alice").to_string()),
            Some("Alice".to_string()),
            Some("ALICE-PC".to_string()),
        )
    }

    #[test]
    fn a_fake_profile_is_removed_in_every_spelling() {
        let scrubber = fake();
        let input = concat!(
            "from C:\\", "Use", "rs\\Alice\\Documents\\scenes.json ",
            "and C:/", "Use", "rs/alice/notes and C:\\", "Use", "rs\\ALICE~1\\logs ",
            "by ALICE on ALICE-PC",
        );
        let out = scrubber.scrub(input);
        assert!(!out.to_ascii_lowercase().contains("alice"), "{out}");
        assert!(!out.contains("ALICE-PC"), "{out}");
        assert!(out.contains(PROFILE_TOKEN), "{out}");
        assert!(out.contains(USER_TOKEN), "{out}");
        assert!(out.contains(MACHINE_TOKEN), "{out}");
    }

    #[test]
    fn the_counted_scrub_counts_every_replacement() {
        let (out, n) = fake().scrub_count(concat!(
            "C:\\", "Use", "rs\\Alice\\x and C:\\", "Use", "rs\\Alice\\y by ALICE-PC"
        ));
        assert!(!out.to_ascii_lowercase().contains("alice"), "{out}");
        assert!(n >= 3, "the two paths and the machine name were replaced: {n}");
    }

    #[test]
    fn an_empty_scrubber_leaves_the_text_alone() {
        let input = concat!("C:\\", "Use", "rs\\Alice\nalice-pc");
        assert_eq!(Scrubber::default().scrub(input), input);
        assert_eq!(Scrubber::default().scrub_count(input), (input.to_string(), 0));
    }

    // The nine original scrubber tests, restored in the move (P8).
    #[test]
    fn a_device_name_in_free_text_is_scrubbed_too() {
        let out = fake().scrub(concat!("ALICE-PC keyboard (C:/", "Use", "rs/ALICE~1/device)"));
        assert_eq!(out, "<machine> keyboard (<profile>/device)");
    }

    #[test]
    fn case_and_slash_variants_do_not_need_a_matching_case() {
        let out = fake().scrub(concat!("c:/", "USERS/alice on alice-pc"));
        assert!(!out.to_ascii_lowercase().contains("alice"), "{out}");
        assert!(!out.to_ascii_lowercase().contains("alice-pc"), "{out}");
    }

    #[test]
    fn a_short_name_with_a_space_is_scrubbed() {
        let scrubber = Scrubber::new(
            Some(concat!("C:\\", "Use", "rs\\Luke Smith").into()),
            Some("Luke Smith".into()),
            None,
        );
        let out = scrubber.scrub(concat!("in C:\\", "Use", "rs\\LUKESM~1\\notes"));
        assert!(!out.contains("LUKESM"), "the real 8.3 name survived: {out}");
        assert!(out.contains(PROFILE_TOKEN), "{out}");
    }

    #[test]
    fn the_real_short_path_is_used_exactly_when_windows_gives_one() {
        let scrubber = Scrubber::with_short(
            Some(concat!("C:\\", "Use", "rs\\Luke Smith").into()),
            None,
            None,
            Some(concat!("C:\\", "Use", "rs\\LUKESM~2").into()),
        );
        let out = scrubber.scrub(concat!(
            "at C:/", "Use", "RS/LUKESM~2/notes and C:\\", "Use", "rs\\LUKESM~1\\old"
        ));
        assert!(!out.contains("LUKESM~2"), "{out}");
        assert!(out.contains(PROFILE_TOKEN), "{out}");
        assert!(out.contains("LUKESM~1"), "{out}");
    }

    #[test]
    fn the_fallback_follows_the_windows_naming_rule() {
        assert_eq!(short_form("Luke Smith"), "LUKESM~1");
        assert_eq!(short_form("John.Doe"), "JOHNDO~1");
        assert_eq!(short_form("Ed"), "ED~1");
    }

    #[test]
    fn a_domain_suffixed_profile_folder_leaves_no_suffix_behind() {
        let scrubber = Scrubber::new(
            Some(concat!("C:\\", "Use", "rs\\Luke.SMITH").to_string()),
            Some("Luke".to_string()),
            None,
        );
        let out = scrubber.scrub("Luke.SMITH pad, and Luke alone");
        assert_eq!(out, "<user> pad, and <user> alone");
    }

    #[test]
    fn bare_names_only_match_whole_names() {
        let scrubber = Scrubber::new(None, Some("Ed".into()), Some("PC".into()));
        let out = scrubber.scrub("collected PCIe Ed PC");
        assert_eq!(out, "collected PCIe <user> <machine>");
        let with_path = Scrubber::new(Some(concat!("C:\\", "Use", "rs\\Ed").into()), None, None);
        assert!(with_path.scrub(concat!("C:\\", "Use", "rs\\Ed\\notes")).contains(PROFILE_TOKEN));
    }

    #[test]
    fn a_device_id_in_error_text_becomes_the_device() {
        let out = scrub_device_ids("the device openrgb:SERIAL-42 refused, and hid:VID_1234 too");
        assert_eq!(out, "the device the device refused, and the device too");
        assert!(!out.contains("SERIAL-42") && !out.contains("VID_1234"), "{out}");
    }

    #[test]
    fn ids_are_taken_whole_in_every_wire_shape() {
        let forms = [
            "device=openrgb:ABC",
            "[openrgb:ABC]",
            "id=\"hid:VID_1234&PID_5678\"",
            "royuan:3151:4015 refused",
            "openrgb:ABC.",
            "openrgb:ABC",
            "openrgb:ABC; next",
        ];
        for form in forms {
            let out = scrub_device_ids(form);
            assert!(!out.contains("ABC"), "{form} -> {out}");
            assert!(!out.contains("3151:4015"), "{form} -> {out}");
            assert!(!out.contains("VID_1234"), "{form} -> {out}");
        }
        assert_eq!(scrub_device_ids("royuan:3151:4015 refused"), "the device refused");
        assert_eq!(scrub_device_ids("openrgb:ABC."), "the device.");
        assert_eq!(scrub_device_ids("openrgb:ABC"), "the device");
        assert_eq!(scrub_device_ids("failed on openrgb:ABC: timeout"), "failed on the device: timeout");
        assert_eq!(scrub_device_ids("openrgb:ABC:"), "the device:");
    }

    /// The `NAME` and value of a same-file `const NAME: &str = "value";`.
    fn string_const(line: &str) -> Option<(String, String)> {
        let at = line.find("const ")?;
        let rest = &line[at + "const ".len()..];
        let name = rest[..rest.find(':')?].trim().to_string();
        let value = rest[rest.find('=')? + 1..].trim_start().strip_prefix('"')?;
        Some((name, value.split('"').next()?.to_string()))
    }

    /// Replaces `{CONST}` references in a `DeviceId(format!("..."))` literal with same-file consts.
    fn expand_consts(literal: &str, consts: &std::collections::BTreeMap<String, String>) -> String {
        let mut out = literal.to_string();
        for (name, value) in consts {
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }

    /// One source line's production DeviceIds: literal prefixes and `{CONST}:` references.
    fn prefixes_on_line(
        line: &str,
        consts: &std::collections::BTreeMap<String, String>,
        found: &mut std::collections::BTreeSet<String>,
    ) {
        for marker in ["DeviceId(format!(\"", "DeviceId(\""] {
            if let Some(i) = line.find(marker) {
                let literal = line[i + marker.len()..].split('"').next().unwrap_or("");
                let expanded = expand_consts(literal, consts);
                if let Some((prefix, _)) = expanded.split_once(':') {
                    let prefix = prefix.trim_end_matches(':');
                    if !prefix.is_empty()
                        && prefix.chars().all(|c| {
                            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'
                        })
                    {
                        found.insert(prefix.to_string());
                    }
                }
            }
        }
    }

    /// Scans one file, skipping only items marked `#[cfg(test)]` (a `mod` or `fn` block, brace
    /// matched), so a `#[cfg(test)]` early in a file cannot hide the production ids after it (H3).
    fn production_prefixes(text: &str) -> std::collections::BTreeSet<String> {
        use std::collections::{BTreeMap, BTreeSet};
        let consts: BTreeMap<String, String> = text.lines().filter_map(string_const).collect();
        let mut found = BTreeSet::new();
        enum State {
            Normal,
            Pending,
            Seeking,
            Skipping(usize),
        }
        let mut state = State::Normal;
        for line in text.lines() {
            let trimmed = line.trim_start();
            let opens = line.matches('{').count();
            let closes = line.matches('}').count();
            match state {
                State::Normal => {
                    if trimmed.starts_with("#[cfg(test)]") {
                        state = State::Pending;
                    } else {
                        prefixes_on_line(line, &consts, &mut found);
                    }
                }
                State::Pending => {
                    if trimmed.starts_with('#') {
                        continue; // another attribute on the same test item
                    }
                    let is_test_item = trimmed.starts_with("mod ")
                        || trimmed.starts_with("fn ")
                        || trimmed.starts_with("async fn ")
                        || trimmed.starts_with("pub fn ")
                        || trimmed.starts_with("pub async fn ");
                    if !is_test_item {
                        state = State::Normal;
                        prefixes_on_line(line, &consts, &mut found);
                    } else if opens > closes {
                        state = State::Skipping(opens - closes);
                    } else {
                        state = State::Seeking; // a signature that spans lines
                    }
                }
                State::Seeking => {
                    if opens > closes {
                        state = State::Skipping(opens - closes);
                    }
                }
                State::Skipping(depth) => {
                    if closes >= depth {
                        state = State::Normal;
                    } else {
                        state = State::Skipping(depth + opens - closes);
                    }
                }
            }
        }
        found
    }

    #[test]
    fn every_adapter_prefix_in_the_adapters_is_scrubbed() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let roots = [crates.join("canvas-adapters/src"), crates.join("canvas-daemon/src")];
        let mut found = std::collections::BTreeSet::new();
        fn walk(dir: &std::path::Path, found: &mut std::collections::BTreeSet<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, found);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    found.extend(production_prefixes(&std::fs::read_to_string(&path).unwrap()));
                }
            }
        }
        for root in &roots {
            walk(root, &mut found);
        }
        // `fake` appears only in test-only DeviceIds, which still reach error text through the
        // fakes; `cpu` is a fan source name, never a DeviceId. Both stay explicit (H3).
        found.insert("fake".to_string());
        found.insert("cpu".to_string());
        // Prove the scan sees the three const/literal shapes that regressed before.
        for required in ["openrgb", "ring", "eightbitdo-ultimate-2", "chione", "royuan", "attack-shark-x6"] {
            assert!(found.contains(required), "the guard did not find {required} in source");
        }
        for prefix in found {
            assert!(ADAPTER_PREFIXES.contains(&prefix.as_str()), "missing adapter prefix {prefix:?}");
        }
    }

    #[test]
    fn a_cfg_test_block_does_not_hide_a_later_openrgb_id() {
        // The early `#[cfg(test)] fn moved` in openrgb.rs must not hide `openrgb:{fingerprint}`.
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let text = std::fs::read_to_string(crates.join("canvas-adapters/src/openrgb.rs")).unwrap();
        assert!(production_prefixes(&text).contains("openrgb"));
    }
}
