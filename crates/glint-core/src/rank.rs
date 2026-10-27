//! Shared scoring for how well a query matches a name.

/// How a query matched, best first. Anything from [`Tier::Initials`] up
/// "starts" a word; when such matches exist, matches inside words are dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Letters in order, e.g. "vscode" in "Visual Studio Code".
    Scattered = 0,
    /// Inside a word: "hell" in "Shell".
    Inside = 1,
    /// First letters of the words: "vsc" for "Visual Studio Code".
    Initials = 2,
    /// Start of a later word: "code" in "Visual Studio Code".
    WordStart = 3,
    /// Start of the name: "hell" in "Hello".
    Prefix = 4,
    Exact = 5,
}

impl Tier {
    pub fn starts_word(self) -> bool {
        self >= Tier::Initials
    }
}

/// Case-insensitive match of `q` (already lowercase) against `name`. Higher
/// scores are better and every tier outranks all lower tiers.
pub fn score(name: &str, q: &str) -> Option<(Tier, i32)> {
    if q.is_empty() {
        return Some((Tier::Exact, 0));
    }
    let lower = name.to_lowercase();
    let len_penalty = lower.chars().count() as i32;
    let make = |tier: Tier, fine: i32| Some((tier, tier as i32 * 10_000 + fine.clamp(-9_999, 9_999)));

    if lower == q {
        return make(Tier::Exact, 0);
    }
    if lower.starts_with(q) {
        return make(Tier::Prefix, -len_penalty);
    }
    if let Some(i) = word_start(&lower, q) {
        return make(Tier::WordStart, -(i as i32) - len_penalty);
    }
    if initials_match(&lower, q) {
        return make(Tier::Initials, -len_penalty);
    }
    if let Some(i) = lower.find(q) {
        return make(Tier::Inside, -(i as i32) - len_penalty);
    }
    if q.chars().count() < 3 {
        return None;
    }
    let first = q.chars().next()?;
    if !lower.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(first)) {
        return None;
    }
    subsequence(&lower, q).map(|s| (Tier::Scattered, s - len_penalty)).and_then(|(t, s)| make(t, s))
}

/// Byte offset in `lower` where `q` starts a word, if it does.
fn word_start(lower: &str, q: &str) -> Option<usize> {
    lower.match_indices(q).map(|(i, _)| i).find(|&i| {
        lower[..i].chars().next_back().map_or(true, |c| !c.is_alphanumeric())
    })
}

/// Which part of `name` to highlight for query `q`, as a byte range of
/// `name`: the match at a word start if there is one, else the first match.
pub fn highlight(name: &str, q: &str) -> Option<(usize, usize)> {
    let q: Vec<char> = q.trim().to_lowercase().chars().collect();
    if q.is_empty() {
        return None;
    }
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    let matches_at = |i: usize| {
        i + q.len() <= chars.len()
            && chars[i..i + q.len()].iter().zip(&q).all(|((_, c), w)| c.to_lowercase().eq(std::iter::once(*w)))
    };
    let mut first = None;
    for i in 0..chars.len() {
        if !matches_at(i) {
            continue;
        }
        let at_word = i == 0 || !chars[i - 1].1.is_alphanumeric();
        if at_word || first.is_none() {
            let start = chars[i].0;
            let end = chars.get(i + q.len()).map_or(name.len(), |(b, _)| *b);
            first = Some((start, end));
            if at_word {
                break;
            }
        }
    }
    first
}

/// "vsc" matches the initials of "visual studio code".
fn initials_match(lower: &str, q: &str) -> bool {
    let mut initials = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|w| w.chars().next());
    let mut qc = q.chars();
    let mut want = qc.next();
    while let Some(w) = want {
        match initials.next() {
            Some(c) if c == w => want = qc.next(),
            Some(_) => {}
            None => return false,
        }
    }
    true
}

/// All query chars appear in order; rewards consecutive runs.
fn subsequence(lower: &str, q: &str) -> Option<i32> {
    let mut chars = lower.chars();
    let mut bonus = 0;
    let mut run = 0;
    for w in q.chars() {
        let mut gap = 0;
        loop {
            let c = chars.next()?;
            if c == w {
                break;
            }
            gap += 1;
        }
        run = if gap == 0 { run + 1 } else { 0 };
        bonus += run * 10 - gap.min(10);
    }
    Some(bonus)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str, q: &str) -> i32 {
        score(name, q).map_or(i32::MIN, |(_, s)| s)
    }

    #[test]
    fn ordering() {
        assert!(s("Code", "code") > s("Codex Launcher", "code"));
        assert!(s("Codex Launcher", "code") > s("Visual Studio Code", "code"));
        assert!(s("Visual Studio Code", "code") > s("Unicode Viewer", "code"));
        assert_eq!(score("Visual Studio Code", "vsc").map(|m| m.0), Some(Tier::Initials));
        assert_eq!(score("Shell", "hell").map(|m| m.0), Some(Tier::Inside));
        assert_eq!(score("Paint", "xyz"), None);
        assert_eq!(score("Calculator", "co"), None);
        assert!(score("Visual Studio Code", "vscode").is_some());
    }

    #[test]
    fn highlights() {
        assert_eq!(highlight("Hello World", "wor"), Some((6, 9)));
        assert_eq!(highlight("Shell Hello", "hell"), Some((6, 10)));
        assert_eq!(highlight("PowerShell", "hell"), Some((6, 10)));
        assert_eq!(highlight("گزارش سالانه", "سال"), Some((11, 17)));
        assert_eq!(highlight("Paint", "x"), None);
    }
}
