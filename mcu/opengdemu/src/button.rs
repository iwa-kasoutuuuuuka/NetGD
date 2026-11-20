//! The front-panel button.
//!
//! One momentary switch to ground on PA20 (`docs/PCB.md`). A press swaps to
//! the next disc of the running game, which is the thing a multi-disc game
//! needs and which nothing else can do: by the time the game asks for disc
//! two, the menu that could have offered a choice was replaced by the game
//! itself. Only the firmware is still running.
//!
//! Polled rather than interrupt-driven. A press is a human-timescale event,
//! so sampling costs nothing, and it keeps a bouncing mechanical contact off
//! the same interrupt path as the FPGA's command line.

use defmt::info;
use embassy_time::{Duration, Timer};

/// SAM3U pin the button is wired to.
const PIN: u8 = 20;

/// How often the pin is sampled.
const POLL: Duration = Duration::from_millis(20);

/// Consecutive agreeing samples needed before a change is believed. Three
/// at 20 ms is 60 ms of settling, comfortably longer than a tactile switch
/// bounces and far shorter than anyone can notice.
const DEBOUNCE: u8 = 3;

pub fn init() {
    // SAFETY: PIOA's clock is on -- the FPGA interrupt line already uses it.
    unsafe { embassy_atsam3::gpio::configure_pioa_input_pullup(PIN) };
    info!("button: PA{} configured as a pulled-up input", PIN);
}

#[embassy_executor::task]
pub async fn button_task() {
    let mut pressed = false;
    let mut agree = 0u8;

    loop {
        Timer::after(POLL).await;

        // Pulled up, switched to ground: low means pressed.
        let now = embassy_atsam3::gpio::pioa_is_low(PIN);
        if now == pressed {
            agree = 0;
            continue;
        }
        agree += 1;
        if agree < DEBOUNCE {
            continue;
        }
        agree = 0;
        pressed = now;

        // Act on the press, not the release: the disc should change when
        // the button goes down, the way every other button on the machine
        // behaves.
        if pressed {
            on_press();
        }
    }
}

fn on_press() {
    // Ejected: the host owns the card (a write may hold a CMD25 open across its
    // waits), and a swap would remount mid-transfer. Do nothing, SD included.
    if !crate::ide::media_present() {
        info!("button: ignored while the media is ejected");
        return;
    }
    // Read the mounted folder's `next` now rather than having remembered it.
    // This is the shallowest caller in the firmware, which is where the work
    // of opening the card belongs.
    let blocks = crate::ide::disk_sectors();
    if let Some(name) = crate::gdi::next_of_mounted(blocks) {
        info!("button: following link to '{}'", name.as_str());
        swap_to(name.as_str());
        return;
    }
    // No link. A numbered folder steps to the next numbered one, as the
    // original GDEMU's button does; anything else does nothing, because
    // swapping to an unrelated disc under a running game would hand it one
    // it never asked for.
    if crate::gdi::mounted_number().is_some() {
        if let Some(name) = crate::gdi::step_folder(blocks, discimg::slot::Step::Next) {
            info!("button: next numbered folder '{}'", name.as_str());
            swap_to(name.as_str());
            return;
        }
    }
    info!("button: pressed, but this image has no 'next' link");
}

fn swap_to(name: &str) {
    crate::ide::request_swap(name);
    // Wake the IDE task: it owns the device state the swap needs,
    // and on an idle bus its interrupt would not otherwise come.
    embassy_atsam3::gpio::pioa_signal_wake();
}
