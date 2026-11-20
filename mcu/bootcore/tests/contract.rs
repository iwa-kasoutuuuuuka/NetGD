//! The resident/application contract (docs/design/resident-app-contract.md,
//! tools/fwimage/FORMAT.md "Resident and application contract"): the GPBR1
//! report, the information block, NGAX, the clock constants, the mapping of
//! recovery errors to results. The vectors are shared with `tools/fwimage`.


use bootcore::app::*;
use bootcore::boot::*;
use bootcore::fat::FatError;
use bootcore::flash::FlashError;
use bootcore::info::*;
use bootcore::ogfw::{ImageError, Reject};
use bootcore::part::*;
use bootcore::update::RecoverError;

fn num(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn cases(text: &'static str) -> impl Iterator<Item = Vec<&'static str>> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().collect())
}

// ------------------------------------------------------------------ GPBR1

/// report.txt: E (encode and parse), R (parse only), N (no mark).
#[test]
fn report_vectors_shared_with_fwimage() {
    let mut n = 0;
    for c in cases(include_str!("../../../tools/fwimage/vectors/report.txt")) {
        match c[0] {
            "E" => {
                let b: Vec<bool> = c[1..7].iter().map(|x| *x == "1").collect();
                let result: u8 = c[7].parse().unwrap();
                let word = num(c[8]);
                let t = Trigger { button: b[0], request: b[1] };
                assert_eq!(report(t, b[2], b[3], b[4], b[5], result), word, "{c:?}");
                let r = Report::parse(word).unwrap();
                assert_eq!(r, Report { trigger: t, wrote: b[2], locked: b[3], slb: b[4], unsealed: b[5], result });
                assert_eq!(r.word(), word);
            }
            "R" => {
                let b: Vec<bool> = c[2..8].iter().map(|x| *x == "1").collect();
                let r = Report::parse(num(c[1])).unwrap();
                assert_eq!(
                    r,
                    Report {
                        trigger: Trigger { button: b[0], request: b[1] },
                        wrote: b[2],
                        locked: b[3],
                        slb: b[4],
                        unsealed: b[5],
                        result: c[8].parse().unwrap()
                    },
                    "{c:?}"
                );
            }
            "N" => assert_eq!(Report::parse(num(c[1])), None, "{c:?}"),
            _ => panic!("{c:?}"),
        }
        n += 1;
    }
    assert!(n >= 20);
}

#[test]
fn report_bits_are_the_documented_ones() {
    assert_eq!(GPBR1_MARK, 0x4EB1);
    assert_eq!(REPORT_MARK_WORD, 0x4EB1_0000);
    assert_eq!(
        [REPORT_BTN, REPORT_REQ, REPORT_WROTE, REPORT_LOCKED, REPORT_SLB, REPORT_UNSEALED],
        [0x100, 0x200, 0x400, 0x800, 0x1000, 0x2000]
    );
    assert_eq!(REPORT_FLAGS_MASK, 0x3F00);
    // the mark's two bytes are 0x4E, 0xB1
    assert_eq!(REPORT_MARK_WORD >> 24, 0x4E);
    assert_eq!((REPORT_MARK_WORD >> 16) & 0xFF, 0xB1);
    assert_eq!(GPBR_RECOVER_MAGIC, 0x4352_474E);
    let all = [
        RESULT_NONE,
        RESULT_RECOVERED,
        RESULT_NO_CARD,
        RESULT_NO_FILE,
        RESULT_READ,
        RESULT_HEADER,
        RESULT_LOCKED,
        RESULT_WRITE,
        RESULT_CRC,
        RESULT_VERIFY,
    ];
    assert_eq!(all, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn report_roundtrips_for_every_flag_set_and_result() {
    for flags in 0u32..64 {
        for result in 0..=255u8 {
            let t = Trigger { button: flags & 1 != 0, request: flags & 2 != 0 };
            let w = report(t, flags & 4 != 0, flags & 8 != 0, flags & 16 != 0, flags & 32 != 0, result);
            assert_eq!(w >> 16, 0x4EB1);
            assert_eq!(w & 0xC000, 0, "reserved bits are written as 0");
            assert_eq!(w & REPORT_FLAGS_MASK, flags << 8);
            assert_eq!(w & 0xFF, result as u32);
            let r = Report::parse(w).unwrap();
            assert_eq!(r.word(), w);
            assert_eq!(r.result, result);
        }
    }
    // reserved bits are ignored when read, and are not carried over
    let r = Report::parse(0x4EB1_C001).unwrap();
    assert_eq!(r.word(), 0x4EB1_0001);
}

#[test]
fn a_cold_gpbr_is_not_a_report_unless_it_looks_like_one() {
    for w in [0u32, 0xFFFF_FFFF, 0x5A5A_A5A5, 0xA5A5_5A5A, GPBR_RECOVER_MAGIC, 0x4EB1_0000 ^ 0x0001_0000] {
        assert_eq!(Report::parse(w), None, "{w:#x}");
    }
    // what a request word is: not a report, and a report is not a request
    assert_ne!(GPBR_RECOVER_MAGIC >> 16, GPBR1_MARK);
    for w in [REPORT_MARK_WORD, REPORT_MARK_WORD | 0x3FFF] {
        assert_ne!(w, GPBR_RECOVER_MAGIC);
    }
}

#[test]
fn request_allowed_rule_x6() {
    let none = Trigger::default();
    let rep = |t: Trigger, res: u8| Some(Report::parse(report(t, false, true, false, false, res)).unwrap());
    // no report (probe start, older resident): allowed
    assert!(request_allowed(None));
    // a normal boot, a button boot, an invalid-app boot: allowed
    assert!(request_allowed(rep(none, RESULT_NONE)));
    assert!(request_allowed(rep(Trigger { button: true, request: false }, RESULT_RECOVERED)));
    // after our own request, whatever came of it: not allowed
    for res in 0..=255u8 {
        assert!(!request_allowed(rep(Trigger { button: false, request: true }, res)));
        assert!(!request_allowed(rep(Trigger { button: true, request: true }, res)));
    }
}

#[test]
fn trigger_of_a_decision() {
    use Action::*;
    assert_eq!(Trigger::of(RunApp, false), Trigger { button: false, request: false });
    assert_eq!(Trigger::of(RunApp, true), Trigger { button: false, request: false }); // cannot happen
    assert_eq!(Trigger::of(Recover { clear_request: false }, false), Trigger { button: false, request: false });
    assert_eq!(Trigger::of(Recover { clear_request: false }, true), Trigger { button: true, request: false });
    assert_eq!(Trigger::of(Recover { clear_request: true }, false), Trigger { button: false, request: true });
    assert_eq!(Trigger::of(Recover { clear_request: true }, true), Trigger { button: true, request: true });
}

/// Every error, every code: an explicit table, and a new variant fails to compile in `result_code`.
#[test]
fn recover_error_to_result() {
    use RecoverError::*;
    let table: Vec<(RecoverError, u8)> = vec![
        (Fat(FatError::Read), RESULT_READ),
        (Fat(FatError::BadChain), RESULT_READ),
        (Fat(FatError::Loop), RESULT_READ),
        (Fat(FatError::Truncated), RESULT_READ),
        (Fat(FatError::NoSignature), RESULT_NO_FILE),
        (Fat(FatError::Gpt), RESULT_NO_FILE),
        (Fat(FatError::NotFat32), RESULT_NO_FILE),
        (Fat(FatError::BadBpb), RESULT_NO_FILE),
        (Fat(FatError::NotFound), RESULT_NO_FILE),
        (Fat(FatError::Empty), RESULT_NO_FILE),
        (NoFile, RESULT_NO_FILE),
        (Header(Reject::BadHeader), RESULT_HEADER),
        (Header(Reject::BadTarget), RESULT_HEADER),
        (Header(Reject::BadDest), RESULT_HEADER),
        (Header(Reject::BadLength), RESULT_HEADER),
        (NoApp, RESULT_HEADER),
        (Image(ImageError::BadDescriptor), RESULT_HEADER),
        (Image(ImageError::Unsealed), RESULT_HEADER),
        (Image(ImageError::BadVector), RESULT_HEADER),
        (Image(ImageError::Truncated), RESULT_CRC),
        (Image(ImageError::CrcMismatch), RESULT_CRC),
        (Image(ImageError::BadTrailer), RESULT_CRC),
        (Changed, RESULT_READ),
        (Locked, RESULT_LOCKED),
        (Flash(FlashError::Refused), RESULT_WRITE),
        (Flash(FlashError::Locked), RESULT_WRITE),
        (Flash(FlashError::Failed), RESULT_WRITE),
        (Flash(FlashError::Timeout), RESULT_WRITE),
        (Verify, RESULT_VERIFY),
    ];
    for (e, want) in table {
        assert_eq!(e.result_code(), want, "{e:?}");
        assert!((RESULT_NO_FILE..=RESULT_VERIFY).contains(&e.result_code()));
    }
}

// ------------------------------------------------------------------ information block

/// info.txt: I (parses), N (version unknown).
#[test]
fn info_vectors_shared_with_fwimage() {
    let mut n = 0;
    for c in cases(include_str!("../../../tools/fwimage/vectors/info.txt")) {
        let b: [u8; 16] = hex(c[1]).try_into().unwrap();
        let got = resident_info(&b);
        match c[0] {
            "I" => {
                let i = got.unwrap_or_else(|| panic!("{c:?}"));
                assert_eq!(
                    (i.gen as u32, i.flags as u32, i.layout_shr8 as u32, i.version, i.crc),
                    (num(c[2]), num(c[3]), num(c[4]), num(c[5]), num(c[6])),
                    "{c:?}"
                );
            }
            "N" => assert_eq!(got, None, "{c:?}"),
            _ => panic!("{c:?}"),
        }
        n += 1;
    }
    assert!(n >= 10);
}

#[test]
fn info_block_position_and_roundtrip() {
    assert_eq!(INFO_ADDR, 0x0008_1FF0);
    assert_eq!(INFO_LEN, 16);
    assert_eq!(INFO_MAGIC, u32::from_le_bytes(*b"NGRS"));
    assert_eq!(IFACE_GEN, 1);
    assert_eq!(RESIDENT_LEN, 0x2000);
    let flags = INFO_F_SELF_LOCK | INFO_F_BUTTON;
    let b = pack_info(IFACE_GEN, flags, 0x2000, version(1, 2, 3), 0xDEAD_BEEF);
    assert_eq!(&b[..4], b"NGRS");
    let i = resident_info(&b).unwrap();
    assert_eq!(i, Info { gen: 1, flags, layout_shr8: 0x20, version: 0x0001_0203, crc: 0xDEAD_BEEF });
    assert_eq!(i.layout(), LAYOUT);
    assert!(!i.dev());
    assert!(resident_info(&pack_info(1, INFO_F_DEV, 0x2000, 0, 0)).unwrap().dev());
    assert_eq!(
        [INFO_F_DEV, INFO_F_SELF_LOCK, INFO_F_BUTTON, INFO_F_LED, INFO_F_WDT_MR],
        [1, 2, 4, 8, 16]
    );
    assert_eq!(version(0, 0, 0), 0);
    assert_eq!(version(255, 255, 255), 0x00FF_FFFF);
}

#[test]
fn resident_region_check_is_the_trailer_rule() {
    let mut region = vec![0xFFu8; RESIDENT_LEN];
    region[..16].copy_from_slice(&[0xA5; 16]);
    let info = pack_info(IFACE_GEN, INFO_F_SELF_LOCK, 0x2000, version(0, 1, 0), 0);
    region[0x1FF0..].copy_from_slice(&info);
    let c = bootcore::crc::crc32(&region[..RESIDENT_LEN - 4]);
    region[RESIDENT_LEN - 4..].copy_from_slice(&c.to_le_bytes());
    assert!(check_region(&region));
    assert!(check_trailer(&region)); // the app's own rule accepts it unchanged
    assert_eq!(resident_info(region[0x1FF0..].try_into().unwrap()).unwrap().crc, c);
    // one bit anywhere, a wrong length
    for off in [0, 100, 0x1FEF, 0x1FF0, 0x1FFB, 0x1FFC, 0x1FFF] {
        let mut r = region.clone();
        r[off] ^= 0x01;
        assert!(!check_region(&r), "offset {off:#x}");
    }
    assert!(!check_region(&region[..RESIDENT_LEN - 1]));
    assert!(!check_region(&[]));
    let mut longer = region.clone();
    longer.push(0);
    assert!(!check_region(&longer));
}

// ------------------------------------------------------------------ NGAX

/// ax.txt: block, bitstream trailer word, verdict.
#[test]
fn ax_vectors_shared_with_fwimage() {
    let mut n = 0;
    for c in cases(include_str!("../../../tools/fwimage/vectors/ax.txt")) {
        let b: [u8; 32] = hex(c[0]).try_into().unwrap();
        let got = match check_ax(&b, num(c[1])) {
            Ok(_) => "OK",
            Err(AxError::NoAx) => "NoAx",
            Err(AxError::BadAx) => "BadAx",
            Err(AxError::StampMismatch) => "StampMismatch",
        };
        assert_eq!(got, c[2], "{c:?}");
        n += 1;
    }
    assert!(n >= 20);
}

#[test]
fn ax_layout_and_generation() {
    assert_eq!(AX_OFF, 0x120);
    assert_eq!(AX_OFF, DESC_OFF + DESC_LEN as u32); // right behind the descriptor
    assert_eq!(AX_LEN, 32);
    assert_eq!(AX_MAGIC, u32::from_le_bytes(*b"NGAX"));
    let b = pack_ax(1, 0xCAFE_1234, 0x55);
    assert_eq!(&b[..4], b"NGAX");
    let a = parse_ax(&b).unwrap();
    assert_eq!(a, Ax { len: 32, min_gen: 1, bs_stamp: 0xCAFE_1234, bs_iface: 0x55 });
    assert!(b[20..].iter().all(|&x| x == 0));
    // design 3-4: min_gen against the resident's generation (0 = unknown resident)
    assert!(a.gen_ok(1));
    assert!(a.gen_ok(2));
    assert!(!a.gen_ok(0));
    assert!(!parse_ax(&pack_ax(2, 0, 0)).unwrap().gen_ok(1));
    assert!(parse_ax(&pack_ax(0, 0, 0)).unwrap().gen_ok(0));
    // the block does not collide with the descriptor, and the descriptor's version is for ever 1
    assert_eq!(DESC_VERSION, 1);
}

// ------------------------------------------------------------------ clock constants (N3)

#[test]
fn clock_constants() {
    assert_eq!(XTAL_HZ, 12_000_000);
    assert_eq!(MCK_HZ, 96_000_000);
    assert_eq!((PLLA_MUL, PLLA_DIV, PLLA_COUNT, MOSC_XTST), (8, 1, 0x3F, 8));
    assert_eq!((FWS_RUN, FWS_WRITE), (4, 6));
    assert_eq!(EEFC_FMR_RUN, 0x0000_0400);
    // the PLLA field the register holds
    assert_eq!(PLLA_MUL - 1, 7);
}

#[test]
fn bootcore_stays_dependency_free() {
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest.split("[dependencies]").nth(1).unwrap().split("\n[").next().unwrap();
    assert!(deps.trim().is_empty(), "bootcore must have no dependencies: {deps:?}");
}
