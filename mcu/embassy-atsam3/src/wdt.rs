
use pac::WDT;

use crate::pac;

#[inline]
const fn wdt() -> &'static pac::wdt::RegisterBlock {
    unsafe { &*WDT::ptr() }
}

/// Turn the watchdog off, for good.
///
/// `WDT_MR` is "read-write once": only a processor reset lets it be written
/// again (datasheet 14.3). This is that one write -- WDDIS, every other field
/// at its reset value. The resident boot region never writes `WDT_MR` in a
/// release build, on purpose (docs/design/resident-app-contract.md W1): the
/// watchdog it inherits is the reset-state one, running with a ~16 s period,
/// which the resident keeps fed only while it recovers, and this write is the
/// application's to make, whether it disables the watchdog (as here) or sets
/// it up for its own use. A development resident does write it, and then this
/// write is silently ignored; either way the watchdog ends up off.
pub fn disable() {
    wdt().mr.write(|w| {
        w.wddis().set_bit()
    });
}