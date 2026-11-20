//! Compress the FPGA bitstream into the firmware image.
//!
//! Which bitstream depends on the board feature: the Cyclone II `.rbf` for
//! `board-v55`, the AG16K `.rbf` for `board-v60` (optional -- see `RBF_V60`).
//! What follows was written for the Cyclone II.
//!
//! The .rbf is already Quartus-compressed about 2:1 (the EP2C5's raw
//! configuration stream is 158,224 bytes), but it is still the single
//! largest thing in flash. Running LZMA over it takes it from ~73.8 KB to
//! ~49.6 KB; `main_task` unpacks it through the `lzma` crate on the fly while
//! clocking it into the FPGA, so the decompressed form never exists anywhere.
//!
//! Quartus compression stays *on* deliberately. Turning it off and letting
//! LZMA work on the raw stream lands about 5 KB smaller, but then the passive
//! serial stream we bit-bang doubles in length (590 kbit -> 1.27 Mbit), and
//! FPGA-ready time is the one thing `main_task` is built around: the board is
//! powered by the Dreamcast, and every millisecond before the FPGA is alive is
//! a millisecond the console sees a dead GD-ROM bus.
//!
//! Compression is done by shelling out to `xz` rather than pulling in an
//! encoder crate: the decoder is fixed to `lc=0 lp=0 pb=0` with a 4 KiB
//! dictionary, and those have to agree exactly on both sides.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Must match `lzma::DICT_SIZE` and the constants baked into the decoder.
const XZ_FILTER: &str = "preset=9e,dict=4KiB,lc=0,lp=0,pb=0";

/// board-v55 (Cyclone II). Required: a missing file fails the build.
const RBF_V55: &str = "../../fpga/Quartus/output_files/OpenGDEMU.rbf";

/// board-v60 (AG16K). Optional: until the first AG16K bitstream exists the
/// firmware is built without one and the loader reports `NoBitstream` at
/// boot. Must be the `.rbf` (bit 0 first on the wire, the loader's default),
/// not the `.bin` -- see `ps-msb-first` in Cargo.toml.
const RBF_V60: &str = "../../fpga/agm/output/OpenGDEMU_ag16k.rbf";

/// The one target both this firmware and the updater are built for.
const TARGET: &str = "thumbv7m-none-eabi";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));

    select_linker_script(&out_dir);

    let v60 = std::env::var_os("CARGO_FEATURE_BOARD_V60").is_some();
    let rbf = if v60 { RBF_V60 } else { RBF_V55 };
    // Also for a file that does not exist (yet): cargo then reruns this
    // script on every build, which is how the AG16K bitstream gets picked up
    // the moment it appears.
    println!("cargo:rerun-if-changed={rbf}");

    let raw = match std::fs::read(rbf) {
        Ok(raw) => Some(raw),
        Err(e) if v60 && e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("cannot read {rbf}: {e} -- build the FPGA project first"),
    };

    let (packed, raw_len) = match raw {
        Some(raw) => {
            if v60 && raw.starts_with(&[0xff, 0x55]) {
                println!(
                    "cargo:warning=board-v60: {rbf} starts with ff 55, which is how a .bin starts (a .rbf has the bits of each byte reversed: ff aa). The loader sends LSB first; use the .rbf, or build with `ps-msb-first` for a .bin"
                );
            }
            let out = Command::new("xz")
                .args(["--format=raw", &format!("--lzma1={XZ_FILTER}"), "-c", rbf])
                .output()
                .expect("`xz` must be on PATH to build the firmware");
            assert!(
                out.status.success(),
                "xz failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            println!(
                "cargo:warning=bitstream: {} -> {} bytes LZMA ({:.2}:1)",
                raw.len(),
                out.stdout.len(),
                raw.len() as f64 / out.stdout.len() as f64
            );
            (out.stdout, raw.len())
        }
        None => {
            println!(
                "cargo:warning=board-v60: no FPGA bitstream at {RBF_V60}; building a firmware that reports `NoBitstream` at boot (LED: 5 blinks) instead of configuring the FPGA"
            );
            (Vec::new(), 0)
        }
    };

    std::fs::write(out_dir.join("bitstream.lzma"), &packed).expect("write bitstream.lzma");
    std::fs::write(
        out_dir.join("bitstream_len.rs"),
        format!(
            concat!(
                "/// Decompressed size of the bitstream, in bytes. The LZMA1 stream is raw\n",
                "/// (no container, no length field), so the decoder is told when to stop.\n",
                "pub const BITSTREAM_RAW_LEN: usize = {};\n",
                "/// False only for a board-v60 build with no bitstream file (see `RBF_V60`).\n",
                "pub const BITSTREAM_PRESENT: bool = {};\n",
            ),
            raw_len,
            raw_len != 0
        ),
    )
    .expect("write bitstream_len.rs");

    build_updater(&out_dir);
}

/// Put the right `memory.x` where `link.x` will find it.
///
/// The two SAM3U parts differ enough in memory map that it is a whole linker
/// script each rather than a couple of defines -- see `link/`. Generating it
/// here instead of checking a single `memory.x` in at the crate root means the
/// part is chosen by a cargo feature and the budget is enforced at link time:
/// pick 2E and the bitstream has to fit the one plane alongside the code, or
/// the build fails with a region overflow instead of the board failing later.
///
/// GNU ld resolves `INCLUDE memory.x` against the current directory before the
/// -L paths, so there must be no `memory.x` at the crate root to shadow this.
fn select_linker_script(out_dir: &Path) {
    let part = if std::env::var_os("CARGO_FEATURE_SAM3U2E").is_some() {
        "sam3u2e"
    } else {
        "sam3u4e"
    };
    let src = format!("link/{part}.x");
    println!("cargo:rerun-if-changed={src}");
    let script = std::fs::read(&src).unwrap_or_else(|e| panic!("cannot read {src}: {e}"));
    std::fs::write(out_dir.join("memory.x"), script).expect("write memory.x");
    println!("cargo:rustc-link-search={}", out_dir.display());
    println!("cargo:warning=part: {part}");

    // defmt's formatting is not free on a part with 28 KB less flash, and
    // DEFMT_LOG is read by defmt's own build script, so it cannot be set from
    // here -- it has to be in the environment of the whole build.
    if part == "sam3u2e" && std::env::var("DEFMT_LOG").as_deref() != Ok("off") {
        println!(
            "cargo:warning=sam3u2e: DEFMT_LOG is not `off`; \
             build with `DEFMT_LOG=off cargo build --release --features sam3u2e` \
             to drop the logging code"
        );
    }
}

/// Build `mcu/fwupdater` and drop its flat binary next to the bitstream.
///
/// It is a separate program rather than a `#[link_section = ".data"]` module
/// so that the flash writer cannot possibly reference anything in flash: it
/// links its own `compiler_builtins`, and its linker script maps SRAM1 and
/// nothing else, so a stray call is a link error instead of a hard fault
/// half-way through erasing the plane it was running from.
fn build_updater(out_dir: &Path) {
    const CRATE: &str = "../fwupdater";
    for f in ["src/main.rs", "src/logic.rs", "updater.x", "Cargo.toml", ".cargo/config.toml"] {
        println!("cargo:rerun-if-changed={CRATE}/{f}");
    }

    let target_dir = out_dir.join("fwupdater");
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(CRATE) // so the crate's own .cargo/config.toml applies
        .args(["build", "--release", "--target", TARGET])
        .arg("--target-dir")
        .arg(&target_dir);
    // Cargo exports the *parent* build's flags to build scripts. Inheriting
    // them here would apply this crate's linker script to the updater and
    // produce something that is not a flat SRAM1 image.
    for leak in [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_TARGET_DIR",
    ] {
        cmd.env_remove(leak);
    }

    let status = cmd.status().expect("failed to run cargo for fwupdater");
    assert!(status.success(), "building {CRATE} failed");

    let blob_path = target_dir.join(TARGET).join("release").join("fwupdater");
    let blob = std::fs::read(&blob_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", blob_path.display()));

    // The blob is copied to the base of SRAM1 and branched into at offset 0.
    assert!(!blob.is_empty(), "fwupdater produced an empty binary");
    assert!(
        blob.len() <= 16 * 1024,
        "fwupdater is {} bytes, larger than SRAM1",
        blob.len()
    );
    // Nothing further to check here: that the updater calls nothing in flash
    // is enforced by its link, not by inspection. `updater.x` maps SRAM1 and
    // nothing else, and the program has no dependencies, so an unresolved
    // call fails the build above rather than reaching this point.

    std::fs::write(out_dir.join("fwupdater.bin"), &blob).expect("write fwupdater.bin");
    println!("cargo:warning=fwupdater: {} bytes", blob.len());
}
