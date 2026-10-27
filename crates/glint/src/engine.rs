//! Ties the indexes to the window: turns a query into rows to display.

use glint_core::rank::highlight;
use glint_core::{App, AppIndex, FileIndex, Kind};
use std::sync::RwLock;

pub const LIMIT: usize = 60;

/// How many of each kind the "All" view shows.
const ALL_APPS: usize = 5;
const ALL_FOLDERS: usize = 4;
const ALL_FILES: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    All,
    Apps,
    Folders,
    Files,
}

impl Category {
    /// Tab order in the window: 0 All, 1 Apps, 2 Folders, 3 Files.
    pub fn from_index(i: i32) -> Self {
        match i {
            0 => Self::All,
            2 => Self::Folders,
            3 => Self::Files,
            _ => Self::Apps,
        }
    }
}

/// One result row, independent of the UI toolkit so it can cross threads.
#[derive(Clone, Debug)]
pub struct Row {
    pub title: String,
    pub subtitle: String,
    /// What to open: a full path, or a `shell:AppsFolder\…` target for apps.
    pub target: String,
    pub kind: RowKind,
    /// Byte range of `title` that matched the query.
    pub matched: Option<(usize, usize)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowKind {
    /// Section title in the "All" view; not selectable.
    Header,
    App,
    Folder,
    File,
}

impl Row {
    fn header(title: &str) -> Self {
        Row { title: title.into(), subtitle: String::new(), target: String::new(), kind: RowKind::Header, matched: None }
    }

    /// Short kind label shown on the right of the row.
    pub fn tag(&self) -> String {
        match self.kind {
            RowKind::Header => String::new(),
            RowKind::App => "App".into(),
            RowKind::Folder => "Folder".into(),
            RowKind::File => match self.title.rsplit_once('.') {
                Some((stem, ext)) if !stem.is_empty() && ext.len() <= 8 => format!("{} file", ext.to_ascii_uppercase()),
                _ => "File".into(),
            },
        }
    }
}

#[derive(Default)]
pub struct Engine {
    pub apps: RwLock<AppIndex>,
    pub files: RwLock<FileIndex>,
}

impl Engine {
    pub fn search(&self, query: &str, category: Category) -> Vec<Row> {
        let mut rows = match category {
            Category::Apps => self.apps(query, LIMIT),
            Category::Folders => self.files(query, Kind::Folder, LIMIT),
            Category::Files => self.files(query, Kind::File, LIMIT),
            Category::All => {
                let mut rows = Vec::new();
                let sections = [
                    ("Apps", self.apps(query, ALL_APPS)),
                    ("Folders", if query.trim().is_empty() { Vec::new() } else { self.files(query, Kind::Folder, ALL_FOLDERS) }),
                    ("Files", if query.trim().is_empty() { Vec::new() } else { self.files(query, Kind::File, ALL_FILES) }),
                ];
                for (title, hits) in sections {
                    if !hits.is_empty() {
                        rows.push(Row::header(title));
                        rows.extend(hits);
                    }
                }
                rows
            }
        };
        for r in &mut rows {
            if r.kind != RowKind::Header {
                r.matched = highlight(&r.title, query);
            }
        }
        rows
    }

    fn apps(&self, query: &str, limit: usize) -> Vec<Row> {
        let apps = self.apps.read().unwrap();
        apps.search(query, limit)
            .into_iter()
            .map(|a: &App| Row {
                title: a.name.clone(),
                subtitle: String::new(),
                target: a.target.clone(),
                kind: RowKind::App,
                matched: None,
            })
            .collect()
    }

    fn files(&self, query: &str, kind: Kind, limit: usize) -> Vec<Row> {
        let files = self.files.read().unwrap();
        files
            .search(query, Some(kind), limit)
            .into_iter()
            .map(|h| {
                let path = files.path(h.id);
                let parent = path.rsplit_once('\\').map_or("", |(p, _)| p).to_string();
                Row {
                    title: files.name(h.id).to_string(),
                    subtitle: parent,
                    target: path,
                    kind: if h.kind == Kind::Folder { RowKind::Folder } else { RowKind::File },
                    matched: None,
                }
            })
            .collect()
    }

    pub fn record_launch(&self, row: &Row) {
        if row.kind == RowKind::App {
            self.apps.write().unwrap().record_launch(&row.target);
        }
    }
}
