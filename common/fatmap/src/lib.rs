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
//! Everything takes the bytes or the FAT lookup as an argument; the firmware
//! supplies the SD reads.
#![cfg_attr(not(test), no_std)]

pub mod extent;
pub mod lfn;
