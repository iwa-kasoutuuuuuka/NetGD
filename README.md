# NetGD

NetGD is a fork of [OpenGDEMU](https://github.com/cepawiel/OpenGDEMU) by Colton Pawielski: open-source firmware and FPGA core for GDEMU-style Dreamcast GD-ROM replacement boards. Its goals are:

1. Run on GDEMU 6.0 boards, which upstream OpenGDEMU does not target.
2. Write game images to the SD card over the Dreamcast Broadband Adapter, which the original GDEMU cannot do.
3. Reach feature parity with the original GDEMU, with better performance.

The plan is in [`docs/PLAN.md`](docs/PLAN.md). None of this has been verified on hardware yet.

### Warning:
At the moment this is a WIP. DO NOT ATTEMPT to flash unless you are okay with having a non-functional
GDEMU (or clone) since I do not have a way to revert to the stock firmware. Even the notes aren't going
to be 100% perfect, I've tried to put little questions when I can't remember perfectly and hope to erase
some of those questions as time goes on.

### Required Pre-Reqs
- #### rust
    - The MCU firmware (`mcu/opengdemu`); `rustup` picks up the toolchain
        and the thumbv7m-none-eabi target from its rust-toolchain.toml
- #### podman (or docker)
    - Generating Verilog from Amaranth
    - Running Quartus 2
        - This program is a nightmare to get working locally on any modern
            linux machine. Lots of broken dependencies and old static libraries,
            so avoid trying a local install.
            Open source bitstream generation when :'(
        - Build the image from `Containers/QuartusII_x64`, then compile with
            `fpga/build_fpga.sh`. No license file is needed: 13.0.1 Web Edition
            is the free tier, and Cyclone II is in it.
        - The installer tarball is gitignored and 4.5 GB. Put it in
            `Containers/QuartusII_x64/` and the build uses it; leave it out and
            the build downloads it. Altera's CDN refuses anything that does not
            look like a browser, so the Containerfile's wget sends a full browser
            header block -- if you fetch it by hand and get a 403, that is why.

### Current Status
The MCU firmware is now Rust (`mcu/opengdemu`, embassy). The list below describes what the code does.
None of it has been checked on a Dreamcast yet; the hardware bring-up plan is in `docs/PLAN.md`.

- **Boot:** the FPGA bitstream is LZMA-compressed in flash and unpacked straight into the passive-serial
  loader, so the FPGA is configured before anything else (`main.rs`). Then the SD card comes up, the MBR and
  FAT32 volume are read, the default image is mounted, IP.BIN is prefetched, and the single IDE task starts.
- **GD-ROM (primary device):** an ATAPI packet device. It answers TOC and session queries, serves
  `CD_READ`/`CD_READ2` from GDI or CDI images on the SD card (PIO and DMA), and plays CD audio through the
  FPGA's serialiser (`cdda.rs`). Eject and load use the standard `START STOP UNIT` with LoEj. Image parsing is
  in `gdi.rs` and `cdi.rs`.
- **Secondary IDE device:** a plain ATA disk that exposes the whole SD card, MBR included
  (`size_disk_from_mbr`, `handle_disk_read`, `handle_disk_write`). It supports PIO and DMA, LBA28 and LBA48.
  Writes are refused while a GD-ROM image is mounted, so the host has to eject first.
- **Image selection:** vendor command `0xE0` (IMAGE_CTL) mounts an image by folder name and reports the
  firmware version (`handle_image_ctl`). Images are named, not numbered.
- **Disc swap button:** a press on PA20 follows the `next` link in the mounted folder's `GAME.CFG` and swaps
  the disc (`button.rs`).
- **Firmware update:** vendor opcode `0xF9` on the secondary device, behind a two-step key. The firmware copies
  a RAM-resident updater (`mcu/fwupdater`) into SRAM1 and branches to it (`fwupdate.rs`).
- **FPGA (`fpga/amaranth`):** two `IDEDevice` instances on the shared G1 bus, plus the CDDA output. Simulation
  tests are in `fpga/amaranth/test*.py`.

Open question: the bit-flip symptoms on the IDE bus described in the older notes have not been re-tested with
this firmware.

### Upstream goals (OpenGDEMU)
The three goals from the original OpenGDEMU plan, and where each one stands:

1. **Eject a GD-ROM image.** Implemented, but not as a custom command. Eject and load go through the standard
   ATAPI `START STOP UNIT` with LoEj (`handle_packet` in `ide.rs`). Choosing which image to load is the
   custom `0xE0` command.
2. **Secondary IDE disk that exposes the SD card, with write support.** Implemented in the firmware
   (`ide.rs`: `handle_disk_read`, `handle_disk_write`, `stage_identify_disk`, `size_disk_from_mbr`). Writes are
   refused while a GD-ROM image is mounted. The disk size comes from the MBR partition table, because the
   card's CSD value was found to undercount the capacity.
3. **Custom command to eject and remount an image by path.** Partly implemented, and on a different device than
   planned. Mounting by folder name is `0xE0` (`handle_image_ctl`), and it is sent as an ATAPI PACKET to the
   GD-ROM (primary device), not to the secondary disk.

### Next Steps
The work list is in `docs/PLAN.md`. Phase 5 (hardware verification) is the one that needs a board.

### Just on the off chance that maybe things go too easily
- Would be pretty cool to compile the Verilog to C with Verilator and hook up to Lxdream-nitro. 
Might be too slow, never used Verilator before this just sounded like a good excuse to try and learn it


### Projects in Use
- [Amaranth](https://github.com/amaranth-lang/amaranth) BSD-2-Clause
- [embassy](https://github.com/embassy-rs/embassy) (MCU firmware)
- [OpenGDEMU](https://github.com/cepawiel/OpenGDEMU) by Colton Pawielski, GPL-3.0 (see `LICENSE`). NetGD is forked from it.

Any omissions are not intentional. Please open an issue if you notice any missed attributions.