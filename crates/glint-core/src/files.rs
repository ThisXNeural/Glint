//! Compact index of every file and folder name, built from the NTFS master
//! file table. Like Everything, names live in one contiguous byte arena so a
//! query is a single memchr-accelerated sweep over memory.
//!
//! Memory per entry: the name bytes + 1 separator + 9 bytes of metadata.

use std::collections::BinaryHeap;
use std::cmp::Reverse;

pub const NO_PARENT: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
}

#[derive(Debug, Clone)]
pub struct FileHit {
    pub id: u32,
    pub kind: Kind,
}

#[derive(Default)]
pub struct FileIndex {
    /// Names separated by `\0`, original case, UTF-8. Renames append, so the
    /// arena is a sequence of slots; dead slots are skipped by search.
    arena: Vec<u8>,
    /// Byte offset of each slot in `arena`, ascending.
    slot_starts: Vec<u32>,
    /// Entry id owning each slot, or `DEAD`.
    slot_ids: Vec<u32>,
    /// Per entry id: its current slot, parent id and flags.
    slots: Vec<u32>,
    parents: Vec<u32>,
    flags: Vec<u8>,
}

const DIR: u8 = 1;
const DELETED: u8 = 2;
const DEAD: u32 = u32::MAX;

impl FileIndex {
    pub fn with_capacity(entries: usize, name_bytes: usize) -> Self {
        Self {
            arena: Vec::with_capacity(name_bytes + entries),
            slot_starts: Vec::with_capacity(entries),
            slot_ids: Vec::with_capacity(entries),
            slots: Vec::with_capacity(entries),
            parents: Vec::with_capacity(entries),
            flags: Vec::with_capacity(entries),
        }
    }

    fn push_slot(&mut self, name: &str, id: u32) -> u32 {
        let slot = self.slot_starts.len() as u32;
        self.slot_starts.push(self.arena.len() as u32);
        self.slot_ids.push(id);
        self.arena.extend_from_slice(name.as_bytes());
        self.arena.push(0);
        slot
    }

    /// Adds an entry and returns its id. `parent` is another id or [`NO_PARENT`].
    pub fn push(&mut self, name: &str, parent: u32, is_dir: bool) -> u32 {
        let id = self.slots.len() as u32;
        let slot = self.push_slot(name, id);
        self.slots.push(slot);
        self.parents.push(parent);
        self.flags.push(if is_dir { DIR } else { 0 });
        id
    }

    /// Changes name and parent in place, so children keep pointing at it.
    pub fn rename(&mut self, id: u32, name: &str, parent: u32) {
        let old = self.slots[id as usize];
        self.slot_ids[old as usize] = DEAD;
        self.slots[id as usize] = self.push_slot(name, id);
        self.parents[id as usize] = parent;
        self.flags[id as usize] &= !DELETED;
    }

    /// Marks an entry deleted; its bytes are reclaimed on the next rebuild.
    pub fn remove(&mut self, id: u32) {
        if let Some(f) = self.flags.get_mut(id as usize) {
            *f |= DELETED;
            self.slot_ids[self.slots[id as usize] as usize] = DEAD;
        }
    }

    pub fn set_parent(&mut self, id: u32, parent: u32) {
        self.parents[id as usize] = parent;
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn memory_bytes(&self) -> usize {
        self.arena.capacity()
            + (self.slot_starts.capacity() + self.slot_ids.capacity() + self.slots.capacity() + self.parents.capacity()) * 4
            + self.flags.capacity()
    }

    pub fn shrink_to_fit(&mut self) {
        self.arena.shrink_to_fit();
        self.slot_starts.shrink_to_fit();
        self.slot_ids.shrink_to_fit();
        self.slots.shrink_to_fit();
        self.parents.shrink_to_fit();
        self.flags.shrink_to_fit();
    }

    pub fn name(&self, id: u32) -> &str {
        let s = self.slot_starts[self.slots[id as usize] as usize] as usize;
        let e = s + memchr::memchr(0, &self.arena[s..]).unwrap();
        // Only ever filled from &str.
        std::str::from_utf8(&self.arena[s..e]).unwrap()
    }

    pub fn kind(&self, id: u32) -> Kind {
        if self.flags[id as usize] & DIR != 0 { Kind::Folder } else { Kind::File }
    }

    /// Full path, rebuilt by walking up the parents.
    pub fn path(&self, id: u32) -> String {
        let mut parts = vec![self.name(id)];
        let mut p = self.parents[id as usize];
        while p != NO_PARENT && parts.len() < 512 {
            parts.push(self.name(p));
            p = self.parents[p as usize];
        }
        parts.reverse();
        parts.join("\\")
    }

    /// Entries whose name contains `query` (ASCII case-insensitive), best
    /// `limit` first: name prefix, then word start, then anywhere; shorter
    /// names first within each group. `only` restricts to folders or files.
    /// Large indexes are swept in parallel, one slice per core.
    pub fn search(&self, query: &str, only: Option<Kind>, limit: usize) -> Vec<FileHit> {
        let q = query.trim().as_bytes();
        if q.is_empty() || limit == 0 || self.is_empty() {
            return Vec::new();
        }
        let n = self.slot_starts.len();
        let threads = std::thread::available_parallelism().map_or(1, |p| p.get()).min(16);
        let mut best: Vec<(i32, u32)> = if threads == 1 || n < 50_000 {
            self.sweep(q, only, limit, 0, n)
        } else {
            let chunk = n.div_ceil(threads);
            std::thread::scope(|s| {
                let handles: Vec<_> = (0..threads)
                    .map(|t| {
                        let (a, b) = (t * chunk, ((t + 1) * chunk).min(n));
                        s.spawn(move || self.sweep(q, only, limit, a, b))
                    })
                    .collect();
                handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
            })
        };
        best.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        best.truncate(limit);
        // Names starting with the query (or a word in them) hide matches
        // inside words, which then only show when nothing starts that way.
        // Tier 2 (word start) and up score above 100_000; tier 1 at most 100_000.
        let starts_word = |score: i32| score > 100_000;
        if best.first().is_some_and(|b| starts_word(b.0)) {
            best.retain(|b| starts_word(b.0));
        }
        best.into_iter().map(|(_, id)| FileHit { id, kind: self.kind(id) }).collect()
    }

    /// Best `limit` (score, id) among slots `from..to`.
    fn sweep(&self, q: &[u8], only: Option<Kind>, limit: usize, from: usize, to: usize) -> Vec<(i32, u32)> {
        if from >= to {
            return Vec::new();
        }
        let first_lo = q[0].to_ascii_lowercase();
        let first_up = q[0].to_ascii_uppercase();
        let want_dir = only.map(|k| k == Kind::Folder);
        let lo = self.slot_starts[from] as usize;
        let hi = self.slot_starts.get(to).map_or(self.arena.len(), |&s| s as usize);
        let arena = &self.arena[..hi];

        // Min-heap of the best `limit` so far.
        let mut heap: BinaryHeap<Reverse<(i32, u32)>> = BinaryHeap::with_capacity(limit + 1);
        let mut slot = from;
        let mut pos = lo;
        while let Some(off) = memchr::memchr2(first_lo, first_up, &arena[pos..]) {
            let at = pos + off;
            pos = at + 1;
            let Some(cand) = arena.get(at..at + q.len()) else { break };
            if !cand.eq_ignore_ascii_case(q) {
                continue;
            }
            // Hits only move forward, so step the entry cursor instead of searching.
            while slot + 1 < to && self.slot_starts[slot + 1] as usize <= at {
                slot += 1;
            }
            let start = self.slot_starts[slot] as usize;
            let id = self.slot_ids[slot];
            // Skip the rest of the name: later hits in it can't score higher.
            let end = at + memchr::memchr(0, &arena[at..]).unwrap();
            pos = end + 1;
            if id == DEAD {
                continue;
            }
            let flags = self.flags[id as usize];
            let wrong_kind = want_dir.is_some_and(|d| (flags & DIR != 0) != d);
            let tier = if at == start {
                3
            } else if !arena[at - 1].is_ascii_alphanumeric() {
                2
            } else {
                1
            };
            if wrong_kind {
                continue;
            }
            let score = tier * 100_000 - ((end - start) as i32).min(99_999);
            if heap.len() < limit {
                heap.push(Reverse((score, id)));
            } else if score > heap.peek().unwrap().0 .0 {
                heap.pop();
                heap.push(Reverse((score, id)));
            }
        }
        heap.into_iter().map(|Reverse(x)| x).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FileIndex {
        let mut ix = FileIndex::default();
        let c = ix.push("C:", NO_PARENT, true);
        let users = ix.push("Users", c, true);
        let docs = ix.push("Documents", users, true);
        ix.push("report-final.docx", docs, false);
        ix.push("Report.pdf", docs, false);
        ix.push("myreport.txt", docs, false);
        ix.push("Monthly report.txt", docs, false);
        ix.push("Reports", users, true);
        ix
    }

    #[test]
    fn ranking_and_paths() {
        let ix = sample();
        let hits = ix.search("report", None, 10);
        let names: Vec<_> = hits.iter().map(|h| ix.name(h.id)).collect();
        assert_eq!(names, ["Reports", "Report.pdf", "report-final.docx", "Monthly report.txt"]);
        assert_eq!(ix.name(ix.search("yrepo", None, 10)[0].id), "myreport.txt");
        assert_eq!(ix.path(hits[1].id), "C:\\Users\\Documents\\Report.pdf");
    }

    #[test]
    fn kind_filter_and_delete() {
        let mut ix = sample();
        let folders = ix.search("rep", Some(Kind::Folder), 10);
        assert_eq!(folders.len(), 1);
        ix.remove(folders[0].id);
        assert!(ix.search("rep", Some(Kind::Folder), 10).is_empty());
        let docs = ix.search("documents", None, 1)[0].id;
        ix.rename(docs, "Papers", 1);
        assert!(ix.search("documents", None, 1).is_empty());
        let pdf = ix.search("report.pdf", None, 1)[0].id;
        assert_eq!(ix.path(pdf), "C:\\Users\\Papers\\Report.pdf");
        assert_eq!(ix.search("REP", Some(Kind::File), 2).len(), 2);
    }

    #[test]
    fn unicode_names() {
        let mut ix = FileIndex::default();
        ix.push("گزارش سالانه.pdf", NO_PARENT, false);
        assert_eq!(ix.search("سالانه", None, 5).len(), 1);
    }
}
