# Vendored crates

## embedded-sdmmc 0.9.0, patched

`embedded-sdmmc-0.9.0/` is the crates.io release of embedded-sdmmc 0.9.0
(MIT OR Apache-2.0; `LICENSE-MIT`, `LICENSE-APACHE` and `NOTICE` are kept),
used through `[patch.crates-io]` in `mcu/netgd/Cargo.toml`. Its changes are
`embedded-sdmmc-0.9.0-opengdemu.patch` (against the pristine copy; the git
history has the pristine copy and each fix as separate commits). Every
changed line is marked `OpenGDEmu patch`.

Left out of the copy: `tests/` and `examples/` (about 750 KB of disk images,
never built for a dependency), `.github/`, `.gitignore` and `.cargo-ok`. The
manifest's `[[example]]` / `[[test]]` targets, which point at the removed
files, are removed by the patch.

### What it fixes

Four specific ways a crafted (or damaged) SD card could panic or hang the
firmware through this crate. The firmware has no watchdog, so a hang never
recovers. These four paths were found by our differential tests and a
review, and are covered by tests. **This does not make the crate robust
against arbitrary corrupt cards**: other paths were not audited, in
particular:

* the write and delete paths (the firmware does not write through this crate);
* first clusters (of files and directories), which are not range-checked
  and can alias another part of the card. That gives wrong data, but no
  panic or hang;
* unchecked arithmetic in other BPB fields, which wraps in release builds.

1. **Long names with lone surrogates** (`filesystem/filename.rs`).
   - **Bug:** `LfnBuffer::push` decoded 13 units plus a carried-over surrogate
     into a `heapless::Vec<char, 13>` and `expect`ed each push. A full piece
     followed by one opening with a lone surrogate gave 14 chars, and the
     14th push panicked. `iterate_dir_lfn` runs on every mount
     (`gdi::walk_to`).
   - **Fix:** capacity 14. Such names list with U+FFFD; all other names
     list as before.
2. **Zero blocks per cluster** (`fat/bpb.rs`).
   - **Bug:** boot-sector byte 13 = 0 divided by zero in `open_volume`.
   - **Fix:** such a BPB, or one claiming less room than its own metadata,
     is refused with `FormatError("Invalid FAT format")`.
3. **Start clusters outside the volume** (`fat/volume.rs`).
   - **Bug:** directory-entry and BPB root clusters are the raw 32-bit
     value. 0x40000002 with 4 blocks per cluster aliased cluster 2, read
     fine, and then `next_cluster` hit its `panic!`.
   - **Fix:** `next_cluster` returns `Error::BadCluster` for any cluster
     outside the data area (no `panic!` left there). As with every other
     broken chain, the crate's directory walks treat that as the end of
     the directory. The aliased first cluster is still read and listed:
     wrong entries, but no panic.
4. **Cyclic directory chains** (`fat/volume.rs`).
   - **Bug:** `iterate_fat16`/`iterate_fat32` (listing) and
     `find_directory_entry` (open by name) followed a looping FAT chain
     forever.
   - **Fix:** each walk ends after as many clusters as the largest legal
     directory can have (65,536 entries = 2 MiB). It ends the way the crate
     ends any broken chain: the listing returns `Ok` with what it has seen,
     possibly repeated, and a look-up returns `NotFound`. An error return
     would cost about 0.9 KB more on the 2E build, because the walk is
     inlined into every caller; this costs about 0.1 KB.

### Tests

`tools/sdmmc-robustness` (run `cd tools/sdmmc-robustness && cargo test`,
with the firmware's release arithmetic: no overflow checks):

* `tests/lfn_panic.rs` calls the crate the way `walk_to` does (384-byte
  `LfnBuffer`, `gamecfg::fit::<96>`). It checks that:
  * the two crashing long names list, and still panic through the
    crates.io release;
  * the walk continues past them;
  * other surrogate placements, a NetGD-shaped card and every
    `mcu/bootcore` fixture list identically with and without the patch.
* `tests/crafted_cards.rs` has:
  * long-name and directory-entry fuzzing;
  * zero blocks per cluster;
  * a root cluster and a folder cluster with the top bits set;
  * a cyclic chain in a FAT32 root and in a FAT16 folder, run under a
    10 s deadline so that a regression cannot stall the suite.

  All of them fail against the crates.io release.

### Upstream status and dropping the patch

As of 2026-11-22, 0.10.0 (the newest release) still has the long-name bug:
same `Vec<char, 13>` and `expect`s, and nothing about it in the changelog.
The other three were not checked against 0.10.0. 0.10.0 is a breaking
release (edition 2024, heapless 0.9, defmt 1.0, iterator callback changes),
so moving to it is not a smaller fix.

Once a release fixes these:

1. Delete `vendor/embedded-sdmmc-0.9.0/` and the `[patch.crates-io]` entry.
2. Bump the version in `mcu/netgd/Cargo.toml`.
3. Point `tools/sdmmc-robustness` at the new version. Its crash and hang
   tests must still pass, and the `upstream` comparison in `lfn_panic.rs`
   can then be removed.
