use discimg::slot::{number_of, Nearest, Step};

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
