//! Installed apps (Start menu shortcuts and Store apps). The list is small
//! (hundreds), so every query scores every app and usage breaks ties.

use crate::rank;

#[derive(Debug, Clone)]
pub struct App {
    pub name: String,
    /// Shortcut path or `shell:AppsFolder\<AUMID>` launch target.
    pub target: String,
    /// How many times the user launched it from search.
    pub launches: u32,
}

#[derive(Default)]
pub struct AppIndex {
    apps: Vec<App>,
}

impl AppIndex {
    pub fn new(mut apps: Vec<App>) -> Self {
        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
        Self { apps }
    }

    pub fn len(&self) -> usize {
        self.apps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    /// Best matches first, at most `limit`. Names that start with the query
    /// (or have a word that does) win outright: "hell" finds "Hello" and,
    /// only when nothing starts that way, "Shell".
    pub fn search(&self, query: &str, limit: usize) -> Vec<&App> {
        let q = query.trim().to_lowercase();
        let mut hits: Vec<(rank::Tier, i32, &App)> = self
            .apps
            .iter()
            .filter_map(|a| {
                let (tier, s) = rank::score(&a.name, &q)?;
                // Frequently launched apps climb within their tier.
                Some((tier, s + (a.launches.min(50) as i32) * 20, a))
            })
            .collect();
        if hits.iter().any(|h| h.0.starts_word()) {
            hits.retain(|h| h.0.starts_word());
        }
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        hits.into_iter().take(limit).map(|(_, _, a)| a).collect()
    }

    pub fn record_launch(&mut self, target: &str) {
        if let Some(a) = self.apps.iter_mut().find(|a| a.target == target) {
            a.launches = a.launches.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, launches: u32) -> App {
        App { name: name.into(), target: name.into(), launches }
    }

    #[test]
    fn usage_breaks_ties() {
        let idx = AppIndex::new(vec![app("Calculator", 0), app("Calendar", 30)]);
        assert_eq!(idx.search("cal", 5)[0].name, "Calendar");
    }

    #[test]
    fn starts_first() {
        let idx = AppIndex::new(vec![app("PowerShell", 40), app("Hello Notes", 0)]);
        let r = idx.search("hell", 5);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "Hello Notes");
        let idx = AppIndex::new(vec![app("PowerShell", 0)]);
        assert_eq!(idx.search("hell", 5).len(), 1);
    }

    #[test]
    fn initials() {
        let idx = AppIndex::new(vec![app("Visual Studio Code", 0), app("Notepad", 0)]);
        let r = idx.search("vsc", 5);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "Visual Studio Code");
    }
}
