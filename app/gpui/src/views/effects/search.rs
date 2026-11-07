//! The effect search, after `app/web/src/components/Search.tsx`: loose
//! matching where the words may come in any order, one letter may be wrong,
//! and a word may match the description. The letters that matched in the
//! name come back as ranges so the card can mark them.

/// The result of matching one effect.
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// The ranges in the name that matched, merged and in order.
    pub ranges: Vec<(usize, usize)>,
    /// A short piece of the description with the match in it, when a word
    /// matched there: (text, ranges in that text).
    pub excerpt: Option<(String, Vec<(usize, usize)>)>,
    /// True when every word matched in the name itself.
    #[allow(dead_code)]
    pub strong: bool,
    /// How well the effect matches: the higher the better.
    pub score: f64,
}

fn words(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Finds a word in the text: exactly, with one letter wrong, or as a loose
/// run of letters. Returns the range it covers.
fn fuzzy_find(text: &str, word: &str) -> Option<(usize, usize)> {
    if word.is_empty() {
        return None;
    }
    if let Some(exact) = text.find(word) {
        return Some((exact, exact + word.len()));
    }
    // One letter wrong.
    if word.len() <= text.len() {
        for i in 0..=(text.len() - word.len()) {
            let mut wrong = 0;
            for k in 0..word.len() {
                if text.as_bytes()[i + k] != word.as_bytes()[k] {
                    wrong += 1;
                    if wrong > 1 {
                        break;
                    }
                }
            }
            if wrong == 1 {
                return Some((i, i + word.len()));
            }
        }
    }
    // A loose run: the letters in order, with gaps between them.
    let mut at = 0;
    let mut start = None;
    for letter in word.chars() {
        let found = text[at..].find(letter)?;
        let absolute = at + found;
        if start.is_none() {
            start = Some(absolute);
        }
        at = absolute + letter.len_utf8();
    }
    start.map(|s| (s, at))
}

/// Merges overlapping ranges so the marks never sit on top of each other.
fn merge(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    if ranges.is_empty() {
        return Vec::new();
    }
    ranges.sort_by_key(|r| r.0);
    let mut out = vec![ranges[0]];
    for range in ranges.into_iter().skip(1) {
        let last = out.last_mut().expect("non-empty");
        if range.0 <= last.1 {
            last.1 = last.1.max(range.1);
        } else {
            out.push(range);
        }
    }
    out
}

/// A short piece of the description with the matched letters in it.
fn excerpt_of(description: &str, ranges: &[(usize, usize)]) -> Option<(String, Vec<(usize, usize)>)> {
    let first = ranges.first()?;
    let from = first.0.saturating_sub(16);
    let to = (first.1 + 30).min(description.len());
    let lead = from > 0;
    let tail = to < description.len();
    let shift = if lead { "…".len() } else { 0 };
    let mut text = String::new();
    if lead {
        text.push('…');
    }
    text.push_str(&description[from..to]);
    if tail {
        text.push('…');
    }
    let ranges = ranges
        .iter()
        .map(|(s, e)| (s - from + shift, e - from + shift))
        .collect();
    Some((text, ranges))
}

/// Matches an effect. Every word has to match somewhere: the name first,
/// then the description. Returns None when a word matches neither.
pub fn match_effect(query: &str, name: &str, description: &str, order: usize) -> Option<MatchResult> {
    let qs = words(query);
    if qs.is_empty() {
        return Some(MatchResult { ranges: Vec::new(), excerpt: None, strong: true, score: 0.0 });
    }
    let lower_name = name.to_lowercase();
    let lower_desc = description.to_lowercase();
    let mut ranges = Vec::new();
    let mut desc_ranges = Vec::new();
    let mut strong = true;
    let mut score = 0.0;
    for word in &qs {
        if let Some(found) = fuzzy_find(&lower_name, word) {
            ranges.push(found);
            score += 10.0;
            if lower_name == *word {
                score += 100.0;
            } else if lower_name.starts_with(word.as_str()) {
                score += 40.0;
            }
            continue;
        }
        if let Some(found) = fuzzy_find(&lower_desc, word) {
            strong = false;
            desc_ranges.push(found);
            score += 2.0;
            continue;
        }
        return None;
    }
    let merged_desc = merge(desc_ranges);
    Some(MatchResult {
        ranges: merge(ranges),
        excerpt: excerpt_of(description, &merged_desc),
        strong,
        score: score - order as f64 * 0.01,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything_in_library_order() {
        let m = match_effect("", "Spiral", "A swirl", 3).expect("matches");
        assert_eq!(m.score, 0.0);
        assert!(m.ranges.is_empty());
        assert!(m.strong);
    }

    #[test]
    fn words_may_come_in_any_order() {
        let a = match_effect("spiral color", "Spiral Color", "x", 0).expect("matches");
        let b = match_effect("color spiral", "Spiral Color", "x", 0).expect("matches");
        assert_eq!(a.score, b.score);
        assert_eq!(a.ranges, b.ranges);
    }

    #[test]
    fn one_letter_wrong_still_matches() {
        let m = match_effect("spirel", "spiral", "x", 0).expect("matches");
        assert_eq!(m.ranges, vec![(0, 6)]);
    }

    #[test]
    fn a_loose_run_matches() {
        let m = match_effect("sprl", "spiral", "x", 0).expect("matches");
        assert_eq!(m.ranges, vec![(0, 6)]);
    }

    #[test]
    fn name_words_score_higher_than_description_words() {
        let name_match = match_effect("spir", "Spiral", "x", 0).expect("matches");
        let desc_match = match_effect("spir", "Other", "a spiral thing", 0).expect("matches");
        assert!(name_match.score > desc_match.score);
        assert!(name_match.strong);
        assert!(!desc_match.strong);
    }

    #[test]
    fn no_word_matching_drops_the_effect() {
        assert!(match_effect("zzzz", "Spiral", "A swirl", 0).is_none());
    }

    #[test]
    fn excerpt_starts_before_and_ends_after_the_match() {
        let description = "01234567890123456789abcdefghijklmnopqrstuv0123456789012345678901234567890";
        let m = match_effect("abcdefgh", "Other", description, 0).expect("matches");
        let (text, _ranges) = m.excerpt.expect("an excerpt");
        assert!(text.contains("abcdefgh"));
        assert!(text.starts_with('…'));
        assert!(text.ends_with('…'));
    }
}
