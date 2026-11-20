use discimg::slot::{number_of, step_accepted, Nearest, Step};

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
