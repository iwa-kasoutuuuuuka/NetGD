use discimg::slot::{mount_target, mount_target_checked, number_of, step_accepted, swap_allowed, Nearest, Step, Target};

#[test]
fn number_of_accepts_two_to_four_digits() {
    assert_eq!(number_of(b"01"), Some(1));
    assert_eq!(number_of(b"99"), Some(99));
    assert_eq!(number_of(b"100"), Some(100));
    assert_eq!(number_of(b"999"), Some(999));
    assert_eq!(number_of(b"1000"), Some(1000));
    assert_eq!(number_of(b"9999"), Some(9999));
}

#[test]
fn number_of_allows_leading_zeros() {
    assert_eq!(number_of(b"001"), Some(1));
    assert_eq!(number_of(b"0012"), Some(12));
}

#[test]
fn number_of_rejects_everything_else() {
    assert_eq!(number_of(b""), None);
    assert_eq!(number_of(b"1"), None);
    assert_eq!(number_of(b"00"), None);
    assert_eq!(number_of(b"000"), None);
    assert_eq!(number_of(b"0000"), None);
    assert_eq!(number_of(b"12345"), None);
    assert_eq!(number_of(b"0A"), None);
    assert_eq!(number_of(b"A1"), None);
    assert_eq!(number_of(b"1 "), None);
    assert_eq!(number_of(b"-1"), None);
    assert_eq!(number_of(b"1.5"), None);
}

fn pick(current: Option<u16>, dir: Step, present: &[u16]) -> Option<u16> {
    let mut n = Nearest::new(current, dir);
    for &p in present {
        n.offer(p);
    }
    n.result()
}

#[test]
fn next_advances_and_wraps() {
    assert_eq!(pick(Some(1), Step::Next, &[1, 2, 3, 4, 5]), Some(2));
    assert_eq!(pick(Some(4), Step::Next, &[5, 4, 3, 2, 1]), Some(5));
    assert_eq!(pick(Some(5), Step::Next, &[1, 2, 3, 4, 5]), Some(1));
    assert_eq!(pick(Some(1), Step::Next, &[1]), Some(1));
}

#[test]
fn prev_retreats_and_wraps() {
    assert_eq!(pick(Some(5), Step::Prev, &[1, 2, 3, 4, 5]), Some(4));
    assert_eq!(pick(Some(2), Step::Prev, &[5, 1, 3, 2, 4]), Some(1));
    assert_eq!(pick(Some(1), Step::Prev, &[1, 2, 3, 4, 5]), Some(5));
    assert_eq!(pick(Some(1), Step::Prev, &[1]), Some(1));
}

#[test]
fn gaps_are_skipped_in_one_pass() {
    // A year-named folder must not make stepping walk every number below it.
    assert_eq!(pick(Some(1), Step::Next, &[2024, 1, 7]), Some(7));
    assert_eq!(pick(Some(7), Step::Next, &[2024, 1, 7]), Some(2024));
    assert_eq!(pick(Some(2024), Step::Next, &[2024, 1, 7]), Some(1));
    assert_eq!(pick(Some(1), Step::Prev, &[2024, 1, 7]), Some(2024));
    assert_eq!(pick(Some(2024), Step::Prev, &[2024, 1, 7]), Some(7));
}

#[test]
fn current_missing_from_the_card() {
    // The mounted folder was removed, or the mounted image is not numbered.
    assert_eq!(pick(Some(4), Step::Next, &[1, 3, 6]), Some(6));
    assert_eq!(pick(Some(4), Step::Prev, &[1, 3, 6]), Some(3));
    assert_eq!(pick(None, Step::Next, &[6, 3, 1]), Some(1));
    assert_eq!(pick(None, Step::Prev, &[1, 6, 3]), Some(6));
    assert_eq!(pick(Some(9), Step::Next, &[1, 3, 6]), Some(1));
    assert_eq!(pick(Some(0), Step::Prev, &[1, 3, 6]), Some(6));
}

#[test]
fn duplicates_and_empty() {
    assert_eq!(pick(Some(1), Step::Next, &[2, 2, 1, 1]), Some(2));
    assert_eq!(pick(None, Step::Next, &[]), None);
    assert_eq!(pick(Some(3), Step::Prev, &[]), None);
}

fn step_ok(current: Option<u16>, dir: Step, present: &[u16], ok: &[u16]) -> Option<u16> {
    step_accepted(
        current,
        dir,
        |n| present.iter().for_each(|&p| n.offer(p)),
        |p| ok.contains(&p),
    )
}

#[test]
fn step_accepted_skips_rejected_like_gaps() {
    let all = [1, 2, 3, 5];
    // 2 and 3 have no image: from 1, Next lands on 5.
    assert_eq!(step_ok(Some(1), Step::Next, &all, &[1, 5]), Some(5));
    // Prev from 5 skips back over them to 1.
    assert_eq!(step_ok(Some(5), Step::Prev, &all, &[1, 5]), Some(1));
    // Wrapping: from 5, Next goes past the end, skips 2/3 and lands on 1.
    assert_eq!(step_ok(Some(5), Step::Next, &all, &[1, 5]), Some(1));
    // Everything acceptable behaves like a plain step.
    assert_eq!(step_ok(Some(1), Step::Next, &all, &all), Some(2));
}

#[test]
fn step_accepted_current_alone_returns_itself() {
    assert_eq!(step_ok(Some(3), Step::Next, &[1, 3, 5], &[3]), Some(3));
    assert_eq!(step_ok(Some(3), Step::Prev, &[1, 3, 5], &[3]), Some(3));
}

#[test]
fn step_accepted_nothing_acceptable_is_none_and_bounded() {
    let mut scans = 0;
    let r = step_accepted(
        Some(2),
        Step::Next,
        |n| {
            scans += 1;
            for p in [1u16, 2, 3, 4] {
                n.offer(p);
            }
        },
        |_| false,
    );
    assert_eq!(r, None);
    assert_eq!(scans, 4, "one scan per slot, then give up");
}

#[test]
fn step_accepted_no_slots_or_no_current() {
    assert_eq!(step_ok(Some(1), Step::Next, &[], &[]), None);
    assert_eq!(step_ok(None, Step::Next, &[4, 2, 9], &[9]), Some(9));
    assert_eq!(step_ok(None, Step::Prev, &[4, 2, 9], &[2, 4]), Some(4));
}

#[test]
fn stepping_from_an_image_less_current_folder() {
    // `52 82 07` onto a folder with no image leaves 07 as the current number
    // (stock: accepted, no medium). The next step must leave it, both ways,
    // and skip other image-less folders on the way (R18 item 1: the old
    // firmware could not move past such a folder).
    let all = [1, 2, 3, 4, 5, 6, 7];
    let ok = [1, 2, 3, 4, 5, 6];
    assert_eq!(step_ok(Some(7), Step::Next, &all, &ok), Some(1));
    assert_eq!(step_ok(Some(7), Step::Prev, &all, &ok), Some(6));
    let all = [1, 3, 7, 8, 9];
    assert_eq!(step_ok(Some(7), Step::Next, &all, &[1, 3, 9]), Some(9));
    assert_eq!(step_ok(Some(8), Step::Prev, &all, &[1, 3, 9]), Some(3));
}

#[test]
fn swap_allowed_while_empty_but_not_while_released() {
    // Mounted: the button and the IDE task's pending swap work.
    assert!(swap_allowed(true, false));
    // No medium because the selected folder has nothing to mount: still the
    // firmware's card, so the button has to work (it is the only way back
    // short of the host).
    assert!(swap_allowed(false, true));
    // Ejected for the host (START/STOP eject, write session, or a disk write
    // seen while empty): the host may hold a CMD25 open, hands off.
    assert!(!swap_allowed(false, false));
}

/// The numbers on a card, as `mount_target`'s `exists`, counting the lookups.
fn target_on(n: u16, card: &[u16], lookups: &mut u32) -> Target {
    mount_target(n, |k| {
        *lookups += 1;
        card.contains(&k)
    })
}

#[test]
fn missing_number_mounts_folder_01() {
    // stock_20261122_09_missing-folder.txt (folders 01-09 only): `52 82 50`
    // is accepted and folder 01 (GDmenu) mounts, with reset_goto = 1 and 2
    // alike (record 10), so it is always 01.
    let card = [1, 2, 3, 4, 5, 6, 7, 8, 9];
    let mut lookups = 0;
    assert_eq!(target_on(50, &card, &mut lookups), Target::Folder(1));
    assert_eq!(lookups, 2, "the number, then 01");
    // 01 is what the firmware remembers (`load_by_name` of 01's name), so the
    // button and `52 81 55/44` continue from 01, not from 50 or the disc
    // before the switch.
    let mounted = Some(1);
    assert_eq!(step_ok(mounted, Step::Next, &card, &card), Some(2));
    assert_eq!(step_ok(mounted, Step::Prev, &card, &card), Some(9));
    // A later switch to a folder that exists is an ordinary one.
    let mut lookups = 0;
    assert_eq!(target_on(3, &card, &mut lookups), Target::Folder(3));
    assert_eq!(lookups, 1);
    // ... and the top of the range falls back the same way.
    assert_eq!(target_on(999, &card, &mut 0), Target::Folder(1));
    assert_eq!(target_on(10, &card, &mut 0), Target::Folder(1));
}

#[test]
fn missing_number_without_folder_01_is_no_medium() {
    // No folder 01 either: accepted, the drive goes empty (D12's handling).
    let card = [2, 3, 7];
    assert_eq!(target_on(50, &card, &mut 0), Target::NoMedium);
    let mut lookups = 0;
    assert_eq!(target_on(1, &card, &mut lookups), Target::NoMedium);
    assert_eq!(lookups, 1, "01 itself is not looked up twice");
    assert_eq!(target_on(1, &[], &mut 0), Target::NoMedium);
}

#[test]
fn numbers_outside_the_fallback_range() {
    // A folder that exists mounts whatever its number (folders 1000-9999 are
    // numbered too). With no folder, numbers past 999 (not probed on stock)
    // and 0 (the default, handled by the caller) are refused.
    assert_eq!(target_on(1000, &[1, 1000], &mut 0), Target::Folder(1000));
    assert_eq!(target_on(1000, &[1, 2], &mut 0), Target::Refused);
    assert_eq!(target_on(65535, &[1], &mut 0), Target::Refused);
    assert_eq!(target_on(0, &[1], &mut 0), Target::Refused);
}

/// R136: a card read error during the lookup is not "no such folder". It is
/// passed up, so the caller refuses the switch and keeps the disc (before
/// R128 a failed lookup refused; as `NoMedium` it unmounted the current disc
/// on a transient SD error).
#[test]
fn read_error_in_the_lookup_is_not_a_missing_folder() {
    let card = [1u16, 2, 3];
    // The error on the number asked for: nothing further is asked.
    let mut asked = vec![];
    let r = mount_target_checked(50, |k| {
        asked.push(k);
        Err::<bool, ()>(())
    });
    assert_eq!(r, Err(()));
    assert_eq!(asked, [50], "no fallback to 01 after an error");
    // The error on the fallback lookup (n is genuinely missing, 01 unreadable).
    let r = mount_target_checked(50, |k| if k == 50 { Ok(false) } else { Err(()) });
    assert_eq!(r, Err(()));
    // No error: the same answers as `mount_target` (R132).
    let ok = |n| mount_target_checked(n, |k| Ok::<_, ()>(card.contains(&k)));
    assert_eq!(ok(2), Ok(Target::Folder(2)));
    assert_eq!(ok(50), Ok(Target::Folder(1)));
    assert_eq!(ok(1000), Ok(Target::Refused));
    let empty = mount_target_checked(50, |_| Ok::<_, ()>(false));
    assert_eq!(empty, Ok(Target::NoMedium));
}
