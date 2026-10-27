//! Synthetic benchmark: 2 million names, typical query timings.
use glint_core::{FileIndex, Kind};
use std::time::Instant;

fn main() {
    let words = ["report", "photo", "invoice", "setup", "readme", "config", "backup", "music",
                 "video", "project", "data", "main", "index", "test", "draft", "عکس", "سند"];
    let exts = ["txt", "pdf", "jpg", "dll", "exe", "json", "rs", "cs", "png", "mp4"];
    let n = 2_000_000u32;
    let t = Instant::now();
    let mut ix = FileIndex::with_capacity(n as usize, n as usize * 18);
    let mut seed = 0x2545F4914F6CDD1Du64;
    let mut rnd = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
    for i in 0..n {
        let r = rnd();
        let is_dir = r % 8 == 0;
        let name = if is_dir {
            format!("{}_{}", words[(r >> 8) as usize % words.len()], r % 1000)
        } else {
            format!("{}{}.{}", words[(r >> 8) as usize % words.len()], r % 10000, exts[(r >> 20) as usize % exts.len()])
        };
        let parent = if i == 0 { u32::MAX } else { (rnd() % i as u64) as u32 };
        ix.push(&name, parent, is_dir);
    }
    ix.shrink_to_fit();
    println!("built {} entries in {:?}, index memory {:.1} MB", ix.len(), t.elapsed(), ix.memory_bytes() as f64 / 1e6);

    for q in ["r", "re", "rep", "repo", "report12", "invoice99.pdf", "zzz", "سند"] {
        for kind in [None, Some(Kind::Folder)] {
            let t = Instant::now();
            let runs = 20;
            let mut hits = 0;
            for _ in 0..runs { hits = ix.search(q, kind, 50).len(); }
            println!("{:>14} {:<8} {:>3} hits  {:>7.2} ms", q, if kind.is_some() { "folders" } else { "all" }, hits,
                     t.elapsed().as_secs_f64() * 1000.0 / runs as f64);
        }
    }
}
