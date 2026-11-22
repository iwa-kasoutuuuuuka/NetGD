//! FAT32 helpers with no I/O, shared by the firmware (`mcu/netgd/src/fat.rs`)
//! and the host tests.
//!
//! * [`lfn`]: recognise a file by its VFAT long name while walking a
//!   directory, 32 bytes at a time. The GDI names a track `Track 01.bin`; the
//!   card stores it as an 8.3 alias (`TRACK0~1.BIN`) that cannot be derived
//!   from the GDI, so the directory has to be searched by long name.
//! * [`extent`]: turn a cluster chain into contiguous block runs, either all
//!   at once or a window at a time.
//!
//! * [`vol`], [`dir`], [`sfn`]: a small read-only FAT32 reader (volume,
//!   directory listing with long names, look-up, file reading with seek, 8.3
//!   names), meant to replace `embedded_sdmmc` in the firmware. Not used by
//!   the firmware yet; `tests/reader.rs` holds it to `embedded_sdmmc`'s
//!   answers. Behind the `reader` feature (FAT16: `fat16`).
//!
//! Everything takes the bytes or the FAT lookup as an argument; the firmware
//! supplies the SD reads.
#![cfg_attr(not(test), no_std)]

#[cfg(feature = "reader")]
pub mod dir;
pub mod extent;
pub mod lfn;
#[cfg(feature = "reader")]
pub mod sfn;
#[cfg(feature = "reader")]
pub mod vol;
