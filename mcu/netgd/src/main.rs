#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::{Spawner, Executor};
use embassy_time::{Duration, Timer};
// use gpio::{Level, Output};
use defmt_rtt as _;
// The pin-finder has its own panic handler (nCONFIG low first): src/pinfind.rs.
#[cfg(not(feature = "pinfind"))]
use panic_probe as _;

use embassy_atsam3::{config::Config, gpio::{Output, Level}, smc, UserPeripherals};
use core::ptr::read_volatile;

use cortex_m_rt::{entry, exception};
use static_cell::StaticCell;

use embedded_hal::digital::v2::ToggleableOutputPin;

mod blockdev;
mod button;
mod cdda;
mod cyclone;
mod cdi;
mod fat;
mod fwupdate;
mod gdi;
mod ide;
mod sdcard;
#[cfg(feature = "pinfind")]
mod pinfind;
#[cfg(feature = "resident")]
mod resident;

static EXECUTOR0: StaticCell<Executor> = StaticCell::new();

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    // Pin-finder: the FPGA off before anything else (LOG R143 item 4).
    #[cfg(feature = "pinfind")]
    pinfind::fpga_off();
    cortex_m::asm::bkpt();
    defmt::panic!("Unhandled exception (IRQn = {})", irqn);
}

#[entry]
fn main() -> ! {
    info!("CPU Booted!!!\n");

    // Clock constants shared with the resident (bootcore::part, contract N3).
    use bootcore::part::{PLLA_DIV, PLLA_MUL, XTAL_HZ};
    let config = Config::new(XTAL_HZ, PLLA_MUL as u16, PLLA_DIV as u8);

    let p = embassy_atsam3::init(config);
    // What the resident said about this boot (contract 6-1). Reads two words
    // of GPBR/flash; the logging is compiled out with DEFMT_LOG=off.
    #[cfg(feature = "resident")]
    resident::log_boot();
   
    let executor0 = EXECUTOR0.init(Executor::new());
    executor0.run(|spawner| {
        spawner.spawn(main_task(spawner, p).unwrap());
    })
    
}

/// FPGA bitstream, LZMA-compressed by `build.rs` and parked in the second
/// flash plane so it does not crowd the code out of the first. Read as
/// ordinary memory-mapped flash at run time and unpacked straight into the
/// passive-serial loader — see `load_fpga`.
#[link_section = ".flash1"]
static BITSTREAM_LZMA: [u8; include_bytes!(concat!(env!("OUT_DIR"), "/bitstream.lzma")).len()] =
    *include_bytes!(concat!(env!("OUT_DIR"), "/bitstream.lzma"));

// `BITSTREAM_RAW_LEN`: the decompressed length, which the raw LZMA1 stream
// does not carry.
include!(concat!(env!("OUT_DIR"), "/bitstream_len.rs"));

/// LED code for "the bitstream does not belong to this code": ten blinks, a
/// pause, three times over (the loader's own codes are `PsError::code`, 1-5).
#[cfg(all(feature = "resident", not(feature = "sam3u2e")))]
async fn blink_stamp_mismatch(led: &mut Output<'static, embassy_atsam3::peripherals::PB18>) {
    for _ in 0..3 {
        for _ in 0..10 {
            led.set_low();
            Timer::after(Duration::from_millis(200)).await;
            led.set_high();
            Timer::after(Duration::from_millis(200)).await;
        }
        Timer::after(Duration::from_millis(2000)).await;
    }
}

#[embassy_executor::task]
// `pinfind` never comes back from `pinfind::run`: the rest is dead code there.
#[cfg_attr(feature = "pinfind", allow(unreachable_code, unused_variables, unused_mut))]
async fn main_task(spawner: Spawner, p: UserPeripherals) {
    // Pin-finder build: every non-PS MCU line back to input + pull-up before
    // the FPGA comes up (src/pinfind.rs).
    #[cfg(feature = "pinfind")]
    pinfind::prepare_pins();

    // Load the FPGA first. When the GDEMU board is powered by the DC
    // every millisecond between MCU power-on and an alive FPGA is one
    // more millisecond the DC sees a dead GD-ROM bus and CDCLK silent.
    // Don't introduce *anything* (LED setup, log banner, timers) before
    // this point that we don't strictly need.
    let t_fpga = embassy_time::Instant::now();
    let mut fpga = cyclone::McuPins::new(
        p.nconfig,
        p.clk,
        p.data,
        p.nstatus,
        p.done,
    );
    // Pin-finder build: the whole run, then halt. Never returns, so nothing
    // below runs: no LED output (PB18), no retries, no IDE side.
    #[cfg(feature = "pinfind")]
    pinfind_main(&mut fpga);

    // Only set up on the failure path below; `main_task` needs it later in
    // any case.
    let mut led_pin = Some(p.led);
    let mut led = None;

    // 4E behind the resident (contract 4-3, V3): the bitstream in plane 1 must
    // be the one this code was built with. Two words compared; the FPGA's own
    // CRC cannot tell an old bitstream from a good one.
    #[cfg(all(feature = "resident", not(feature = "sam3u2e")))]
    if !resident::bitstream_stamp_ok() {
        error!("bitstream stamp mismatch: plane 1 is not the bitstream of this code");
        // Ask the resident to reinstall from the card -- once: when this boot
        // already is the answer to such a request there is nothing more to
        // try, and asking again would reset forever.
        let _ = resident::request_recovery();
        let mut l = Output::new(led_pin.take().unwrap(), Level::High);
        loop {
            blink_stamp_mismatch(&mut l).await;
        }
    }

    let stats = loop {
        let r = if !BITSTREAM_PRESENT {
            Err(psload::LoadFailure { error: psload::PsError::NoBitstream, attempts: 0 })
        } else {
            // The decoder's working memory (a 4 KiB window plus the
            // probability model, ~7.6 KB) is carved out of the CD_READ
            // staging buffer. That buffer is 14 KB of SRAM1 that nothing
            // touches until the first CD_READ, which cannot arrive before
            // the FPGA is even configured -- so the compression costs no
            // static RAM at all. Every attempt re-borrows it and starts the
            // decoder over; the previous attempt's decoder is gone by then.
            cyclone::load(&mut fpga, || {
                let scratch = lzma::Scratch::from_buf(ide::loader_scratch())
                    .expect("CD_READ_BUF too small for the LZMA scratch");
                cyclone::LzSource(lzma::Decoder::new(&BITSTREAM_LZMA, BITSTREAM_RAW_LEN, scratch))
            })
        };
        let f = match r {
            Ok(s) => break s,
            Err(f) => f,
        };

        // The FPGA is not there, and no IDE task is worth running without
        // it: every register the IDE task serves lives in the FPGA, the DC
        // would see a dead GD-ROM bus either way, and a half-configured part
        // may drive the bus with garbage. So halt here -- but not for good:
        // the MCU stays up with the debug port usable (SWD reflash works),
        // the LED says why, and a failure that might be transient (the
        // FPGA's rail or POR came up late) is retried after each blink code.
        // A broken or missing bitstream cannot get better by itself, so
        // those just blink.
        error!("FPGA load failed after {} attempt(s): {}", f.attempts, f.error);
        let l = led.get_or_insert_with(|| Output::new(led_pin.take().unwrap(), Level::High));
        cyclone::blink_code(l, &f.error).await;
        while !f.error.retryable() {
            cyclone::blink_code(l, &f.error).await;
        }
    };

    let mut led = led.unwrap_or_else(|| Output::new(led_pin.take().unwrap(), Level::High));
    // Worth watching: LZMA decoding is interleaved with the bit-bang, so it
    // adds directly to the window where the DC sees a dead GD-ROM bus.
    info!(
        "FPGA Loaded! ({} ms, {} bytes, {} attempt(s), early done: {})",
        t_fpga.elapsed().as_millis(),
        stats.bytes,
        stats.attempts,
        stats.early_done
    );

    // Cyclone II runs internal init after CONF_DONE rises before user
    // logic is live. Without this slack, the post-load probe floats and
    // returns 0xffff.
    Timer::after(Duration::from_millis(10)).await;

    // Quick FPGA bus sanity check — FPGA[12] should read 0xC0DE on both
    // devices. Skipping the verbose register/FIFO sweep we used to do here
    // (regression-tested in Amaranth sim).
    let fpga = smc::fpga_window();
    unsafe {
        let p = read_volatile(fpga.add(12));
        let s = read_volatile(fpga.add(0x80 + 12));
        info!("FPGA debug: primary={:#06x} secondary={:#06x}", p, s);
    }

    // SD card init + FAT32 + GDI parse first, so the IDE task can
    // serve real game data on its first command. We delay the IDE task
    // spawn until the GDI is ready so it can take a `&'static Gdi`.
    match sdcard::init().await {
        Ok(card) => {
            let mb = (card.size_blocks as u64) * 512 / 1_048_576;
            let gb = mb / 1024;
            info!(
                "Card up: {:?}, RCA={:#06x}, {} blocks (~{} GB / {} MB) [CSD-derived]",
                card.capacity, card.rca, card.size_blocks, gb, mb
            );

            #[cfg(feature = "sd-bench")]
            sdcard::bench_read(1_000_000).await;

            // The emulated disk on the secondary exposes the whole card,
            // MBR included, so the host sees a normal partitioned disk and
            // can reach the same filesystem that holds the GD-ROM images.
            ide::size_disk_from_mbr(card.size_blocks);

            let dev = blockdev::HsmciBlockDevice::new();
            dev.set_num_blocks(card.size_blocks);
            // Debug listing of the card (two directory levels). Off by
            // default: it costs ~2.5 KB of flash and boot time even with
            // logging off (LOG R81). Enable with the `sd-list` feature.
            #[cfg(feature = "sd-list")]
            list_root_dir(dev);

            match gdi::load_default(card.size_blocks) {
                Ok(g) => ide::mount_gdi(g),
                Err(e) => warn!("gdi load failed: {:?}", e),
            }
            // Pre-populate the CD_READ buffer with IP.BIN before the BIOS
            // can issue its first CD_READ. Otherwise the BIOS triggers its
            // DMA channel ~50ms before our SD read finishes and the host
            // times out around 10-11 KB into the 14 KB transfer.
            ide::prefetch_mounted().await;
            // PA22 is the FPGA's MCU_IRQn line. Configure it for a
            // falling-edge IRQ then spawn the (single) IDE task that
            // services both emulated devices when the line drops.
            unsafe {
                embassy_atsam3::gpio::enable_pioa_falling_irq(22);
            }
            // CDDA: the SysTick hook feeds the FPGA from a ring the task
            // below keeps filled from the card.
            cdda::init();
            spawner.spawn(cdda::cdda_task().unwrap());
            // The front-panel button: swaps to the next disc of a
            // multi-disc game, which only the firmware can do once the game
            // has replaced the menu.
            button::init();
            spawner.spawn(button::button_task().unwrap());
            spawner.spawn(ide::ide_task(
                ide::IdeDevice::primary(),
                ide::IdeDevice::secondary(),
            ).unwrap());
        }
        Err(e) => {
            warn!("SD init failed: {:?}", e);
            unsafe {
                embassy_atsam3::gpio::enable_pioa_falling_irq(22);
            }
            spawner.spawn(ide::ide_task(
                ide::IdeDevice::primary(),
                ide::IdeDevice::secondary(),
            ).unwrap());
        }
    }

    let fpga = smc::fpga_window();
    // Only read by the `fpga-diag` log below, so they are only declared when
    // it is: without the cfg they are four "assigned but never used" warnings
    // in every ordinary build, and deleting them instead would quietly break
    // the diagnostic's deltas.
    #[cfg(feature = "fpga-diag")]
    let (mut prev_free, mut prev_strobes, mut prev_rstn_edges, mut prev_audio) =
        (0u16, 0u16, 0u16, 0u16);
    loop {
        Timer::after(Duration::from_secs(1)).await;
        led.toggle().unwrap();
        #[cfg_attr(not(feature = "fpga-diag"), allow(unused_variables))]
        let (free_run, strobes, rstn_edges, bus_state, audio_alive) = unsafe {
            (
                read_volatile(fpga.add(14)),
                read_volatile(fpga.add(15)),
                read_volatile(fpga.add(16)),
                read_volatile(fpga.add(17)),
                read_volatile(fpga.add(18)),
            )
        };
        // free_run delta = nonzero proves FPGA core clock is alive.
        // strobes delta = nonzero proves DC is touching the IDE bus.
        // rstn_edges nonzero = DC has issued at least one reset cycle.
        // bus_state bit 0 = ide_rstn level (1 = out of reset).
        // audio_alive delta = nonzero proves the 11.2896 MHz xtal is
        // running and routes into the FPGA fabric (≈344 edges/sec
        // expected at audio_div[15]).
        //
        // Off by default. info! is unsynchronized, so this once-a-
        // second line interleaves mid-word with the IDE task's per-command
        // tracing and shreds both. Enable `fpga-diag` only when the point of
        // the run is FPGA liveness rather than ATAPI behaviour.
        #[cfg(feature = "fpga-diag")]
        info!(
            "diag: free_run=0x{:04x} (d={}) strobes={} (d={}) rstn_edges=0x{:04x} (d=0x{:04x}) bus=0x{:02x} audio=0x{:04x} (d={})",
            free_run,
            free_run.wrapping_sub(prev_free),
            strobes,
            strobes.wrapping_sub(prev_strobes),
            rstn_edges,
            rstn_edges.wrapping_sub(prev_rstn_edges),
            bus_state & 0xFF,
            audio_alive,
            audio_alive.wrapping_sub(prev_audio),
        );
        #[cfg(feature = "fpga-diag")]
        {
            prev_free = free_run;
            prev_strobes = strobes;
            prev_rstn_edges = rstn_edges;
            prev_audio = audio_alive;
        }
    }
}

/// Open MBR partition 0, walk the root directory, log every entry.
/// Lives in its own fn so embedded-sdmmc's `Volume<'_, ...>` borrow of the
/// VolumeManager doesn't leak into `main_task`'s drop scope.
#[cfg(feature = "sd-list")]
fn list_root_dir(dev: blockdev::HsmciBlockDevice) {
    let mgr = embedded_sdmmc::VolumeManager::new(dev, blockdev::DummyClock);
    let volume = match mgr.open_volume(embedded_sdmmc::VolumeIdx(0)) {
        Ok(v) => v,
        Err(e) => {
            warn!("open_volume(0) failed: {:?}", defmt::Debug2Format(&e));
            return;
        }
    };
    info!("opened MBR volume 0");

    let root = match volume.open_root_dir() {
        Ok(r) => r,
        Err(e) => {
            warn!("open_root_dir failed: {:?}", defmt::Debug2Format(&e));
            return;
        }
    };
    info!("--- / ---");
    // First pass: log every entry and remember the first sub-directory's
    // ShortFileName so we can recurse after `iterate_dir` releases its lock.
    let mut subdir: Option<embedded_sdmmc::ShortFileName> = None;
    let _ = root.iterate_dir(|entry| {
        log_entry("/", entry);
        if subdir.is_none()
            && entry.attributes.is_directory()
            && entry.name.base_name() != b"."
            && entry.name.base_name() != b".."
        {
            subdir = Some(entry.name.clone());
        }
    });

    if let Some(name) = subdir {
        match root.open_dir(&name) {
            Ok(sub) => {
                let label = sfn_label(&name);
                info!("--- /{}/ ---", core::str::from_utf8(&label).unwrap_or("?"));
                let _ = sub.iterate_dir(|child| {
                    log_entry(core::str::from_utf8(&label).unwrap_or("?"), child);
                });
            }
            Err(e) => warn!("open_dir failed: {:?}", defmt::Debug2Format(&e)),
        }
    }
}

#[cfg(feature = "sd-list")]
fn sfn_label(name: &embedded_sdmmc::ShortFileName) -> [u8; 12] {
    let mut out = [b' '; 12];
    let raw = name.base_name();
    let n = raw.len().min(8);
    out[..n].copy_from_slice(&raw[..n]);
    out
}

#[cfg(feature = "sd-list")]
fn log_entry(parent: &str, entry: &embedded_sdmmc::DirEntry) {
    let mut name = [b' '; 12];
    let raw = entry.name.base_name();
    let n = raw.len().min(8);
    name[..n].copy_from_slice(&raw[..n]);
    let ext = entry.name.extension();
    if !ext.is_empty() {
        name[8] = b'.';
        let m = ext.len().min(3);
        name[9..9 + m].copy_from_slice(&ext[..m]);
    }
    let name_str = core::str::from_utf8(&name).unwrap_or("?");
    info!(
        "{}/{}  {} bytes  dir={}",
        parent,
        name_str,
        entry.size,
        entry.attributes.is_directory()
    );
}

/// `pinfind`: hand the PS pins and the embedded bitstream to the pin-finder
/// (src/pinfind.rs). Never returns. LOG R143 items 3 and 4.
#[cfg(feature = "pinfind")]
fn pinfind_main(fpga: &mut cyclone::McuPins) -> ! {
    pinfind::main(fpga, BITSTREAM_PRESENT, BITSTREAM_RAW_LEN, || {
        let scratch = lzma::Scratch::from_buf(ide::loader_scratch())
            .expect("CD_READ_BUF too small for the LZMA scratch");
        cyclone::LzSource(lzma::Decoder::new(&BITSTREAM_LZMA, BITSTREAM_RAW_LEN, scratch))
    })
}
