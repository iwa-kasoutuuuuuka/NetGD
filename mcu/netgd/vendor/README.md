# Vendored crates

## embedded-sdmmc 0.9.0 (one-line fix)

`embedded-sdmmc-0.9.0/` is the crates.io release of embedded-sdmmc 0.9.0
(MIT OR Apache-2.0; `LICENSE-MIT`, `LICENSE-APACHE` and `NOTICE` are kept),
used through `[patch.crates-io]` in `mcu/netgd/Cargo.toml`. It carries one
change, `embedded-sdmmc-0.9.0-opengdemu.patch`.
The upstream `tests/`, `examples/`, `.github/`, `.gitignore` and `.cargo-ok` are
left out (the first two hold
about 750 KB of disk images; none is built for a dependency). The manifest's
`[[example]]`/`[[test]]` targets for them are removed by the patch.
The history keeps the pristine copy and the fix as separate commits.

### Why

`LfnBuffer::push` decodes each 13-unit long-name piece, plus the unpaired
surrogate it carried over from the previous piece, into a
`heapless::Vec<char, 13>` and `expect`s every push.
A full 13-unit piece followed by a piece that starts with a lone surrogate
decodes to 14 chars. The 14th push panics ("Vec was full?!").
Two names that trigger it:

* 13 chars, then a lone low surrogate, then more;
* 25 chars, then a lone low surrogate and a lone high surrogate.

The firmware lists directories with `iterate_dir_lfn` (`gdi::walk_to`, on
every mount). So a card holding such a name, which a PC can write, would
panic the firmware. The bug was found by the host differential tests of
the fatmap reader (LOG R165).

### The fix

The capacity of the Vec goes from 13 to 14, which is the most the decode can
produce. Such a name now lists the way embedded-sdmmc already treats lone
surrogates elsewhere: U+FFFD in place of the surrogate. Every other name
lists exactly as before.

`tools/sdmmc-lfn-check` tests this, calling the crate the way `walk_to` does
(384-byte `LfnBuffer`, `gamecfg::fit::<96>`). It checks:

* both names list correctly through the patched copy and still panic
  through the crates.io release;
* the walk continues past them;
* other surrogate placements, a full NetGD-shaped card and every
  `mcu/bootcore` fixture list identically with and without the patch.

Run it with `cd tools/sdmmc-lfn-check && cargo test`.

### Upstream status and dropping the patch

As of 2026-11-22 the bug is still present in 0.10.0, the newest release:
`char_vec` is still `Vec<char, 13>` with the same `expect`s, and the
changelog does not mention it.
0.10.0 is also a breaking release (edition 2024, heapless 0.9, defmt 1.0,
iterator callback changes), so moving to it is not a smaller fix.

Once a release fixes `LfnBuffer::push`:

1. Delete `vendor/embedded-sdmmc-0.9.0/` and the `[patch.crates-io]` entry.
2. Bump the `embedded-sdmmc` version in `mcu/netgd/Cargo.toml`.
3. Point `tools/sdmmc-lfn-check`'s `embedded-sdmmc` at the new version.
   Its two crash tests must still pass, and the `upstream` comparison can
   then be removed.
