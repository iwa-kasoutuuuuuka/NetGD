
use pac::EFC0;
#[cfg(not(feature = "sam3u2e"))]
use pac::EFC1;

use crate::pac;
use bootcore::part::FWS_RUN;

#[inline]
const fn efc0() -> &'static pac::efc0::RegisterBlock{
    unsafe { &*EFC0::ptr() }
}

/// The SAM3U2E has one flash plane and one EEFC; EFC1 does not exist there, so
/// nothing may touch it (a write to the hole is at best ignored).
#[cfg(not(feature = "sam3u2e"))]
#[inline]
const fn efc1() -> &'static pac::efc1::RegisterBlock{
    unsafe { &*EFC1::ptr() }
}


pub unsafe fn init() {
    // The same wait states the resident hands over with (bootcore::part).
    efc0().fmr.write(|w| unsafe {
        w.fws().bits(FWS_RUN)
    });
    #[cfg(not(feature = "sam3u2e"))]
    efc1().fmr.write(|w| unsafe {
        w.fws().bits(FWS_RUN)
    });
}