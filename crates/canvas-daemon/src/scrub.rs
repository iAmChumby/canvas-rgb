//! The one scrubber for text that can leave the app (#574).
//!
//! The daemon is the only part that can read the profile path, the user name and the machine
//! name, so this is where they are removed. It runs LAST over every string in the shareable
//! report, before any length cap, and the same function is reused by the diagnostics zip (#575)
//! and safe start (#577). The editor never rebuilds a scrubbed string.
//!
//! Every spelling is covered: backslash and forward slash, the 8.3 short form (`LUKE~1`), and
//! any case. Names are read at runtime, never hard-coded.

/// What replaces the profile path, in every spelling.
pub const PROFILE_TOKEN: &str = "<profile>";
/// What replaces the user name.
pub const USER_TOKEN: &str = "<user>";
/// What replaces the machine name.
pub const MACHINE_TOKEN: &str = "<machine>";

/// A set of replacements, longest first, so a longer path goes before the user name inside it.
#[derive(Debug, Clone, Default)]
pub struct Scrubber {
    patterns: Vec<(String, &'static str)>,
}

impl Scrubber {
    /// Reads the three names Windows gives the process. Missing values simply add no pattern;
    /// a scrubber with fewer names still removes the ones it knows.
    pub fn from_env() -> Self {
        Self::new(
            std::env::var("USERPROFILE").ok(),
            std::env::var("USERNAME").ok(),
            std::env::var("COMPUTERNAME").ok(),
        )
    }

    /// Builds the patterns from explicit values. Tests pass a fake profile here so no test
    /// depends on the account that happens to run it.
    pub fn new(profile: Option<String>, user: Option<String>, machine: Option<String>) -> Self {
        let mut patterns: Vec<(String, &'static str)> = Vec::new();
        if let Some(profile) = profile.filter(|p| !p.trim().is_empty()) {
            let profile = profile.trim().trim_end_matches(['\\', '/']).to_string();
            for path in spelling_variants(&profile) {
                patterns.push((path, PROFILE_TOKEN));
            }
            // The path's own last component and its short form also occur on their own, e.g. in
            // a device name a vendor put there.
            if let Some(last) = profile.rsplit(['\\', '/']).next().filter(|s| !s.is_empty()) {
                push_name(&mut patterns, last, USER_TOKEN);
            }
        }
        if let Some(user) = user.filter(|u| !u.trim().is_empty()) {
            push_name(&mut patterns, user.trim(), USER_TOKEN);
        }
        if let Some(machine) = machine.filter(|m| !m.trim().is_empty()) {
            push_name(&mut patterns, machine.trim(), MACHINE_TOKEN);
        }
        patterns.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        patterns.dedup_by(|a, b| a.0 == b.0);
        Self { patterns }
    }

    /// Removes every known name from one string. The same function serves the report, the zip
    /// and safe start; callers must not write their own replacement.
    pub fn scrub(&self, input: &str) -> String {
        let mut out = input.to_string();
        for (needle, replacement) in &self.patterns {
            out = replace_case_insensitive(&out, needle, replacement);
        }
        out
    }

    /// True when there is nothing to remove (e.g. Windows gave no names at all).
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

/// Adds the long name, its short 8.3 form and those forms with either slash direction.
fn push_name(patterns: &mut Vec<(String, &'static str)>, name: &str, token: &'static str) {
    for form in [name.to_string(), short_form(name)] {
        patterns.push((form, token));
    }
}

/// `Alice` also appears as `ALICE~1` in 8.3 paths; both are the same person.
fn short_form(component: &str) -> String {
    let upper = component.to_ascii_uppercase();
    let head: String = upper.chars().take(6).collect();
    format!("{head}~1")
}

/// Both slash directions for a path, with the long and the 8.3 form of its last component.
fn spelling_variants(path: &str) -> Vec<String> {
    let components: Vec<String> = path
        .split(['\\', '/'])
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let mut variants = Vec::new();
    for separator in ['\\', '/'] {
        let sep = separator.to_string();
        variants.push(components.join(&sep));
        if components.len() > 1 {
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

/// ASCII-case-insensitive replace. Windows paths and names are compared byte by byte; a
/// multi-byte UTF-8 sequence has no ASCII case, so byte offsets stay on character boundaries.
fn replace_case_insensitive(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() || needle.len() > haystack.len() {
        return haystack.to_string();
    }
    let bytes = haystack.as_bytes();
    let pattern = needle.as_bytes();
    let mut out = String::with_capacity(haystack.len());
    let mut at = 0;
    while at + pattern.len() <= bytes.len() {
        if bytes[at..at + pattern.len()]
            .iter()
            .zip(pattern)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            out.push_str(replacement);
            at += pattern.len();
        } else {
            // Any byte of a UTF-8 sequence is >= 0x80; copying one byte at a time would split a
            // character, so copy the whole character when the first byte is not ASCII.
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
    // The tail is shorter than the pattern and was never examined: it still belongs to the text.
    out.push_str(&haystack[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> Scrubber {
        Scrubber::new(
            Some(r"C:\Users\Alice".to_string()),
            Some("Alice".to_string()),
            Some("ALICE-PC".to_string()),
        )
    }

    #[test]
    fn a_fake_profile_is_removed_in_every_spelling() {
        let scrubber = fake();
        let input = concat!(
            "from C:\\Users\\Alice\\Documents\\scenes.json ",
            "and C:/Users/alice/notes and C:\\Users\\ALICE~1\\logs ",
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
    fn a_device_name_in_free_text_is_scrubbed_too() {
        let out = fake().scrub("ALICE-PC keyboard (C:/Users/ALICE~1/device)");
        assert_eq!(out, "<machine> keyboard (<profile>/device)");
    }

    #[test]
    fn an_empty_scrubber_leaves_the_text_alone() {
        let input = "C:\\Users\\Alice\nalice-pc";
        assert_eq!(Scrubber::default().scrub(input), input);
    }

    #[test]
    fn case_and_slash_variants_do_not_need_a_matching_case() {
        let out = fake().scrub("c:/USERS/alice on alice-pc");
        assert!(!out.to_ascii_lowercase().contains("alice"), "{out}");
        assert!(!out.to_ascii_lowercase().contains("alice-pc"), "{out}");
    }
}
