//! Cluster chains to runs, the eager map and the on-demand window.

use fatmap::extent::*;

const G: ClusterGeom = ClusterGeom { data_lba: 1000, blocks_per_cluster: 64 }; // 32 KB clusters
const CB: u32 = 64 * 512;

/// A FAT as a vector: `fat[c]` is the next cluster, 0 for end of chain.
struct Fat {
    next: Vec<u32>,
    reads: std::cell::Cell<usize>,
}

impl Fat {
    fn new(n: usize) -> Fat {
        Fat { next: vec![0; n], reads: Default::default() }
    }
    /// Lay a file over `clusters` in order.
    fn chain(&mut self, clusters: &[u32]) {
        for w in clusters.windows(2) {
            self.next[w[0] as usize] = w[1];
        }
    }
    fn lookup(&self) -> impl FnMut(u32) -> Result<Option<u32>, ()> + '_ {
        move |c| {
            self.reads.set(self.reads.get() + 1);
            match self.next.get(c as usize) {
                None => Err(()),
                Some(0) => Ok(None),
                Some(&n) => Ok(Some(n)),
            }
        }
    }
}

fn full_map(fat: &Fat, first: u32, size: u32) -> Vec<(u32, Extent)> {
    let mut out = Vec::new();
    let r = walk(&G, Cursor::start(first, size).unwrap(), fat.lookup(), |o, e| {
        out.push((o, e));
        true
    });
    assert_eq!(r, Walk::Done);
    out
}

/// Deterministic xorshift.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A file of `clusters` clusters placed in runs of random length at random
/// non-overlapping places. Returns (fat, first cluster, the cluster list).
fn fragmented(rng: &mut Rng, clusters: usize, mean_run: u64) -> (Fat, u32, Vec<u32>) {
    let total = clusters * 3 + 100;
    let mut fat = Fat::new(total + 2);
    let mut list = Vec::new();
    // Hand out runs from shuffled slots so neighbouring runs are rarely adjacent.
    let mut next_free = 2u32;
    while list.len() < clusters {
        let len = (1 + rng.below(mean_run * 2)) as usize;
        let len = len.min(clusters - list.len());
        let start = next_free;
        list.extend(start..start + len as u32);
        // Leave a gap of at least 1, so runs never merge by accident.
        next_free = start + len as u32 + 1 + rng.below(3) as u32;
    }
    fat.chain(&list);
    (fat, list[0], list)
}

/// Reference: where block `b` of the file is, from the cluster list.
fn ref_block(list: &[u32], block: u32) -> u32 {
    let c = list[(block / G.blocks_per_cluster) as usize];
    G.cluster_lba(c) + block % G.blocks_per_cluster
}

#[test]
fn start_rejects_empty_and_reserved() {
    assert!(Cursor::start(0, 100).is_none());
    assert!(Cursor::start(1, 100).is_none());
    assert!(Cursor::start(5, 0).is_none());
    assert_eq!(Cursor::start(5, 1).unwrap().blocks_left, 1);
    assert_eq!(Cursor::start(5, 512).unwrap().blocks_left, 1);
    assert_eq!(Cursor::start(5, 513).unwrap().blocks_left, 2);
    assert_eq!(Cursor::start(5, u32::MAX).unwrap().blocks_left, 8_388_608);
}

#[test]
fn contiguous_file_is_one_run() {
    let mut fat = Fat::new(100);
    fat.chain(&[10, 11, 12, 13]);
    let m = full_map(&fat, 10, 4 * CB);
    assert_eq!(m, vec![(0, Extent { lba: G.cluster_lba(10), blocks: 256 })]);
}

#[test]
fn partial_last_cluster_counts_only_its_blocks() {
    let mut fat = Fat::new(100);
    fat.chain(&[10, 11, 12]);
    let m = full_map(&fat, 10, 2 * CB + 3 * 512 + 1);
    assert_eq!(m, vec![(0, Extent { lba: G.cluster_lba(10), blocks: 128 + 4 })]);
}

#[test]
fn fragments_are_split_at_every_jump_including_backwards() {
    let mut fat = Fat::new(100);
    fat.chain(&[10, 11, 40, 41, 42, 7, 90]);
    let m = full_map(&fat, 10, 7 * CB);
    assert_eq!(
        m,
        vec![
            (0, Extent { lba: G.cluster_lba(10), blocks: 128 }),
            (2 * CB, Extent { lba: G.cluster_lba(40), blocks: 192 }),
            (5 * CB, Extent { lba: G.cluster_lba(7), blocks: 64 }),
            (6 * CB, Extent { lba: G.cluster_lba(90), blocks: 64 }),
        ]
    );
}

#[test]
fn chain_ending_early_is_broken() {
    let mut fat = Fat::new(100);
    fat.chain(&[10, 11, 12]); // 3 clusters
    let r = walk(&G, Cursor::start(10, 5 * CB).unwrap(), fat.lookup(), |_, _| true);
    assert_eq!(r, Walk::Broken);
}

#[test]
fn bad_fat_entries_are_broken() {
    let mut fat = Fat::new(100);
    fat.next[10] = 1; // reserved
    let r = walk(&G, Cursor::start(10, 2 * CB).unwrap(), fat.lookup(), |_, _| true);
    assert_eq!(r, Walk::Broken);
    // A link past the table: the lookup itself fails.
    let mut fat = Fat::new(100);
    fat.next[10] = 11;
    let r = walk(&G, Cursor::start(10, 3 * CB).unwrap(), |c| if c == 11 { Err(()) } else { fat.lookup()(c) }, |_, _| true);
    assert_eq!(r, Walk::Broken);
}

#[test]
fn looping_chain_terminates() {
    let mut fat = Fat::new(100);
    fat.next[10] = 20;
    fat.next[20] = 10;
    let mut n = 0;
    let r = walk(&G, Cursor::start(10, 100 * CB).unwrap(), fat.lookup(), |_, _| {
        n += 1;
        true
    });
    assert_eq!(r, Walk::Done);
    assert_eq!(n, 100); // every hop is a jump; bounded by the file's size
}

#[test]
fn a_file_that_ends_exactly_on_a_cluster_needs_no_chain_end() {
    // The last cluster's FAT entry is never read: a corrupt end-of-chain
    // marker there cannot hurt.
    let mut fat = Fat::new(100);
    fat.chain(&[10, 11]);
    fat.next[11] = 1;
    let m = full_map(&fat, 10, 2 * CB);
    assert_eq!(m.len(), 1);
}

#[test]
fn refused_run_resumes_without_loss() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for round in 0..50 {
        let n = 5 + rng.below(300) as usize;
        let mean = 1 + rng.below(6);
        let (fat, first, _) = fragmented(&mut rng, n, mean);
        let size = n as u32 * CB - rng.below(CB as u64) as u32;
        let whole = full_map(&fat, first, size);
        // Take them 3 at a time.
        let mut got = Vec::new();
        let mut cur = Cursor::start(first, size).unwrap();
        loop {
            let mut batch = 0;
            let r = walk(&G, cur, fat.lookup(), |o, e| {
                if batch == 3 {
                    return false;
                }
                batch += 1;
                got.push((o, e));
                true
            });
            match r {
                Walk::Done => break,
                Walk::More(c) => cur = c,
                Walk::Broken => panic!("round {round}"),
            }
        }
        assert_eq!(got, whole, "round {round}");
    }
}

#[test]
fn locate_walks_runs_end_to_end() {
    let ex = [Extent { lba: 100, blocks: 4 }, Extent { lba: 500, blocks: 2 }, Extent { lba: 900, blocks: 8 }];
    assert_eq!(locate(&ex, 0, 0), Some((100, 0, 4)));
    assert_eq!(locate(&ex, 0, 511), Some((100, 511, 4)));
    assert_eq!(locate(&ex, 0, 512 * 3 + 7), Some((103, 7, 1)));
    assert_eq!(locate(&ex, 0, 2048), Some((500, 0, 2)));
    assert_eq!(locate(&ex, 0, 2048 + 512 + 1), Some((501, 1, 1)));
    assert_eq!(locate(&ex, 0, 3072), Some((900, 0, 8)));
    assert_eq!(locate(&ex, 0, 3072 + 8 * 512 - 1), Some((907, 511, 1)));
    assert_eq!(locate(&ex, 0, 3072 + 8 * 512), None);
    // With a base offset (a window that starts mid-file).
    assert_eq!(locate(&ex, 4096, 4095), None);
    assert_eq!(locate(&ex, 4096, 4096), Some((100, 0, 4)));
    assert_eq!(locate(&[], 0, 0), None);
}

#[test]
fn locate_does_not_overflow_near_4_gib() {
    let ex = [Extent { lba: 0, blocks: 8_388_608 }]; // 4 GiB exactly
    assert_eq!(locate(&ex, 0, u32::MAX), Some((8_388_607, 511, 1)));
}

#[test]
fn eager_map_and_locate_agree_with_the_chain() {
    let mut rng = Rng(77);
    for _ in 0..30 {
        let n = 3 + rng.below(200) as usize;
        let mean = 1 + rng.below(8);
        let (fat, first, list) = fragmented(&mut rng, n, mean);
        let size = n as u32 * CB - rng.below(CB as u64) as u32;
        let m = full_map(&fat, first, size);
        let exts: Vec<Extent> = m.iter().map(|&(_, e)| e).collect();
        // file offsets reported by the walk are the prefix sums
        let mut off = 0u32;
        for (o, e) in &m {
            assert_eq!(*o, off);
            off += e.blocks * 512;
        }
        for _ in 0..200 {
            let b = rng.below(((size as u64) + 511) / 512) as u32;
            let (lba, intra, contig) = locate(&exts, 0, b * 512 + 5).unwrap();
            assert_eq!(lba, ref_block(&list, b));
            assert_eq!(intra, 5);
            assert!(contig >= 1);
            // the whole contiguous stretch really is contiguous
            for k in 0..contig.min(70) {
                assert_eq!(ref_block(&list, b + k), lba + k);
            }
        }
        assert_eq!(locate(&exts, 0, (((size as u64 + 511) / 512) * 512).min(u32::MAX as u64) as u32 ), None);
    }
}

#[test]
fn window_agrees_with_the_map_under_random_access() {
    let mut rng = Rng(4242);
    for n_window in [1usize, 2, 4, 16] {
        let n = 400;
        let (fat, first, list) = fragmented(&mut rng, n, 3);
        let size = n as u32 * CB - 1000;
        macro_rules! check {
            ($N:expr) => {{
                let mut w: Window<$N> = Window::new();
                for _ in 0..3000 {
                    // mostly nearby offsets, sometimes far jumps
                    let b = rng.below(((size as u64) + 511) / 512) as u32;
                    let (lba, intra, contig) =
                        w.lookup(1, &G, first, size, b * 512 + 9, fat.lookup()).unwrap();
                    assert_eq!(lba, ref_block(&list, b));
                    assert_eq!(intra, 9);
                    assert!(contig >= 1);
                }
            }};
        }
        match n_window {
            1 => check!(1),
            2 => check!(2),
            4 => check!(4),
            _ => check!(16),
        }
    }
}

#[test]
fn streaming_a_file_walks_its_chain_once() {
    let mut rng = Rng(9);
    let n = 2000;
    let (fat, first, list) = fragmented(&mut rng, n, 5);
    let size = n as u32 * CB;
    let mut w: Window<8> = Window::new();
    let mut off = 0u32;
    while (off as u64) < size as u64 {
        let (lba, _, contig) = w.lookup(0, &G, first, size, off, fat.lookup()).unwrap();
        assert_eq!(lba, ref_block(&list, off / 512));
        off += contig * 512; // what the data path does: take the whole run
    }
    // One FAT read per cluster hop at most (plus re-walks of a refused run),
    // nowhere near the quadratic cost of walking from the start every time.
    let reads = fat.reads.get();
    assert!(reads < n * 2, "{reads} FAT reads for {n} clusters");
}

#[test]
fn small_reads_inside_the_window_cost_no_fat_reads() {
    let mut fat = Fat::new(100);
    fat.chain(&[10, 11, 12, 40, 41]);
    let size = 5 * CB;
    let mut w: Window<4> = Window::new();
    w.lookup(3, &G, 10, size, 0, fat.lookup()).unwrap();
    let before = fat.reads.get();
    for b in 0..(5 * 64) {
        let (lba, _, _) = w.lookup(3, &G, 10, size, b * 512, fat.lookup()).unwrap();
        let c = if b < 192 { 10 + b / 64 } else { 40 + (b - 192) / 64 };
        assert_eq!(lba, G.cluster_lba(c) + b % 64);
    }
    assert_eq!(fat.reads.get(), before);
}

#[test]
fn window_switches_files_and_survives_invalidate() {
    let mut fat = Fat::new(200);
    fat.chain(&[10, 11, 12]);
    fat.chain(&[100, 101, 150]);
    let mut w: Window<4> = Window::new();
    let a = w.lookup(1, &G, 10, 3 * CB, 2 * CB, fat.lookup()).unwrap();
    assert_eq!(a.0, G.cluster_lba(12));
    let b = w.lookup(2, &G, 100, 3 * CB, 2 * CB, fat.lookup()).unwrap();
    assert_eq!(b.0, G.cluster_lba(150));
    let a2 = w.lookup(1, &G, 10, 3 * CB, 0, fat.lookup()).unwrap();
    assert_eq!(a2.0, G.cluster_lba(10));
    w.invalidate();
    // Chain edited behind the cache's back: invalidate makes it re-read.
    fat.next[11] = 0;
    fat.next[10] = 12;
    let a3 = w.lookup(1, &G, 10, 2 * CB, CB, fat.lookup()).unwrap();
    assert_eq!(a3.0, G.cluster_lba(12));
}

#[test]
fn window_past_the_end_and_broken_chains() {
    let mut fat = Fat::new(100);
    fat.chain(&[10, 11, 12]);
    let mut w: Window<2> = Window::new();
    assert!(w.lookup(1, &G, 10, 3 * CB, 3 * CB, fat.lookup()).is_none());
    assert!(w.lookup(1, &G, 10, 3 * CB, 3 * CB - 1, fat.lookup()).is_some());
    // size says 5 clusters, chain has 3
    assert!(w.lookup(1, &G, 10, 5 * CB, 4 * CB, fat.lookup()).is_none());
    // and the window stays usable afterwards
    assert!(w.lookup(1, &G, 10, 3 * CB, 0, fat.lookup()).is_some());
    // empty file / first cluster 0
    assert!(w.lookup(1, &G, 0, 0, 0, fat.lookup()).is_none());
}

/// Not a test of behaviour: prints how many extents a track file needs as
/// fragmentation grows, against the firmware's pool, for the notes in
/// docs/research/g3-read-path.md. Run with `-- --nocapture`.
#[test]
fn capacity_report() {
    let mut rng = Rng(2024);
    // A 1.2 GB track at 32 KB clusters, a 100 MB one, a 20 MB one.
    println!("mean run (clusters) | 1.2 GB track | 100 MB track | 20 MB track  (extents, median of 9)");
    for mean in [100_000u64, 10_000, 1_000, 300, 100, 30, 10, 3] {
        let mut row = Vec::new();
        for mb in [1200u64, 100, 20] {
            let n = (mb * 1024 * 1024 / CB as u64) as usize;
            let mut counts = Vec::new();
            for _ in 0..9 {
                let (fat, first, _) = fragmented_fast(&mut rng, n, mean);
                counts.push(full_map_len(&fat, first, n as u32 * CB));
            }
            counts.sort();
            row.push(counts[4]);
        }
        println!("{:>19} | {:>12} | {:>12} | {:>12}", mean, row[0], row[1], row[2]);
    }
}

fn fragmented_fast(rng: &mut Rng, clusters: usize, mean_run: u64) -> (Fat, u32, Vec<u32>) {
    let mut fat = Fat::new(clusters * 2 + 100);
    let mut prev: Option<u32> = None;
    let mut first = 0;
    let mut next_free = 2u32;
    let mut done = 0usize;
    while done < clusters {
        let len = ((1 + rng.below(mean_run * 2)) as usize).min(clusters - done);
        let start = next_free;
        for k in 0..len as u32 {
            let c = start + k;
            if let Some(p) = prev {
                fat.next[p as usize] = c;
            } else {
                first = c;
            }
            prev = Some(c);
        }
        done += len;
        next_free = start + len as u32 + 1;
    }
    (fat, first, Vec::new())
}

fn full_map_len(fat: &Fat, first: u32, size: u32) -> usize {
    let mut n = 0;
    walk(&G, Cursor::start(first, size).unwrap(), fat.lookup(), |_, _| {
        n += 1;
        true
    });
    n
}

// ---- R89 C7: a damaged FAT value must not overflow the LBA ----------------

#[test]
fn cluster_lba_is_checked() {
    assert_eq!(G.checked_cluster_lba(2), Some(1000));
    assert_eq!(G.checked_cluster_lba(3), Some(1064));
    assert_eq!(G.checked_cluster_lba(1), None);
    assert_eq!(G.checked_cluster_lba(0), None);
    // 0x0FFFFFF6 * 64 overflows u32.
    assert_eq!(G.checked_cluster_lba(0x0FFF_FFF6), None);
    assert_eq!(G.checked_cluster_lba(u32::MAX), None);
}

#[test]
fn chain_leaving_the_block_space_is_broken_not_wrapped() {
    let fat = Fat::new(100);
    let mut emitted = Vec::new();
    let r = walk(
        &G,
        Cursor::start(10, 3 * CB).unwrap(),
        |c| if c == 10 { Ok(Some(0x0FFF_FFF6)) } else { fat.lookup()(c) },
        |o, e| {
            emitted.push((o, e));
            true
        },
    );
    assert_eq!(r, Walk::Broken);
    assert_eq!(emitted.len(), 1); // the cluster before the jump was fine
    // A first cluster that is already out of range.
    let r = walk(&G, Cursor::start(0x0FFF_FFF6, CB).unwrap(), fat.lookup(), |_, _| true);
    assert_eq!(r, Walk::Broken);
    // A run whose end passes u32::MAX.
    let g = ClusterGeom { data_lba: u32::MAX - 100, blocks_per_cluster: 64 };
    let r = walk(&g, Cursor::start(2, 2 * CB).unwrap(), |c| Ok(Some(c + 1)), |_, _| true);
    assert_eq!(r, Walk::Broken);
}
