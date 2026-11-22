//! Host tests for the frame decoder: the line code at many clock rates, with
//! drift, jitter and noise, through either polarity.

use pinfind::*;

/// A tiny deterministic PRNG (xorshift32), so the tests need no crates.
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// Uniform in [-amp, amp].
    fn jitter(&mut self, amp: i64) -> i64 {
        if amp == 0 {
            0
        } else {
            (self.next() as i64 % (2 * amp + 1)) - amp
        }
    }
}

/// Turn frames into electrical edges `(time, high)`.
///
/// `unit` is the tick length at the start, `drift_ppm` changes it per tick
/// (positive = clock slows down), `jitter` moves each edge by up to that many
/// time units. `active_high` models an inverter in the path.
struct Line {
    t: u64,
    unit: f64,
    drift: f64,
    jitter: i64,
    active_high: bool,
    rng: Rng,
    edges: Vec<(u32, bool)>,
}

impl Line {
    fn new(unit: f64) -> Self {
        Line { t: 1000, unit, drift: 0.0, jitter: 0, active_high: false, rng: Rng(0x1234_5678), edges: vec![] }
    }
    fn level(&self, active: bool) -> bool {
        active == self.active_high
    }
    fn pulse(&mut self, active: bool, ticks: u32) {
        // An edge at the start of the pulse (only if the level changes).
        let lvl = self.level(active);
        if self.edges.last().map(|e| e.1) != Some(lvl) {
            let j = self.rng.jitter(self.jitter);
            let at = (self.t as i64 + j).max(0) as u64;
            self.edges.push((at as u32, lvl));
        }
        let mut d = 0.0;
        for _ in 0..ticks {
            d += self.unit;
            self.unit *= 1.0 + self.drift;
        }
        self.t += d.round() as u64;
    }
    fn idle(&mut self, ticks: u32) {
        self.pulse(false, ticks);
    }
    fn frame(&mut self, bytes: &[u8]) {
        let mut ps = vec![];
        encode(bytes, |a, n| ps.push((a, n)));
        for (a, n) in ps {
            self.pulse(a, n);
        }
    }
    fn index_frame(&mut self, idx: u8) {
        let mut b = [0u8; MAX_FRAME_BYTES];
        let n = frame_bytes(idx, None, &mut b);
        self.frame(&b[..n]);
    }
}

/// Run a decoder over the edges, polling between them and at the end.
fn decode(edges: &[(u32, bool)], active_high: bool, end: u32) -> (Vec<Frame>, u32) {
    let mut d = Decoder::new(active_high);
    let mut out = vec![];
    for &(t, h) in edges {
        if let Some(f) = d.poll(t.wrapping_sub(1)) {
            out.push(f);
        }
        if let Some(f) = d.edge(t, h) {
            out.push(f);
        }
    }
    if let Some(f) = d.poll(end) {
        out.push(f);
    }
    (out, d.dropped)
}

fn indices(fs: &[Frame]) -> Vec<u8> {
    fs.iter().map(|f| f.index()).collect()
}

#[test]
fn every_index_at_one_rate() {
    let mut l = Line::new(50.0);
    l.idle(20);
    for i in 0..=255u8 {
        l.index_frame(i);
        l.idle(10);
    }
    let end = l.t as u32 + 10_000;
    let (fs, dropped) = decode(&l.edges, false, end);
    assert_eq!(indices(&fs), (0..=255u8).collect::<Vec<_>>());
    assert_eq!(dropped, 0);
}

#[test]
fn rate_does_not_matter() {
    // From 3 time units per tick (a logic analyser barely oversampling) to
    // 100k (a slow oscillator seen by a 96 MHz cycle counter).
    for unit in [3.0, 4.0, 7.0, 16.0, 100.0, 1234.0, 25_000.0, 100_000.0] {
        let mut l = Line::new(unit);
        l.idle(8);
        for i in [0u8, 1, 0x5a, 0xa5, 0xff, 37] {
            l.index_frame(i);
        }
        let (fs, dropped) = decode(&l.edges, false, l.t as u32 + 1_000_000);
        assert_eq!(indices(&fs), vec![0, 1, 0x5a, 0xa5, 0xff, 37], "unit {unit}");
        assert_eq!(dropped, 0, "unit {unit}");
        // The tick estimate is close to the truth.
        let u = fs[0].unit as f64;
        assert!((u - unit).abs() <= unit * 0.15 + 1.0, "unit {unit} estimated {u}");
    }
}

#[test]
fn back_to_back_frames_end_on_next_sync() {
    // No extra idle between frames: the 4T gap is closed by the next SYNC's
    // falling edge, not by poll.
    let mut l = Line::new(20.0);
    l.idle(5);
    for i in 10..20u8 {
        l.index_frame(i);
    }
    let mut d = Decoder::new(false);
    let mut got = vec![];
    for &(t, h) in &l.edges {
        if let Some(f) = d.edge(t, h) {
            got.push(f.index());
        }
    }
    // All but the last come out of edges alone.
    assert_eq!(got, (10..19u8).collect::<Vec<_>>());
    assert_eq!(d.poll(l.t as u32).map(|f| f.index()), Some(19));
}

#[test]
fn drift_within_a_frame() {
    // 0.5 % per tick slows the clock by ~1.5x over a 2-byte frame (~70 ticks)
    // and the decoder still follows it through the 1T idles.
    for drift in [0.005, -0.004] {
        let mut l = Line::new(200.0);
        l.drift = drift;
        l.idle(5);
        for i in [0x00u8, 0xff, 0x81, 0x7e] {
            l.index_frame(i);
            l.unit = 200.0; // each frame starts at the nominal rate again
        }
        let (fs, _) = decode(&l.edges, false, l.t as u32 + 100_000);
        assert_eq!(indices(&fs), vec![0x00, 0xff, 0x81, 0x7e], "drift {drift}");
    }
}

#[test]
fn jitter_of_a_quarter_tick() {
    let mut l = Line::new(40.0);
    l.jitter = 10;
    l.idle(5);
    let want: Vec<u8> = (0..64).map(|i| (i * 37 + 11) as u8).collect();
    for &i in &want {
        l.index_frame(i);
    }
    let (fs, dropped) = decode(&l.edges, false, l.t as u32 + 10_000);
    assert_eq!(indices(&fs), want);
    assert_eq!(dropped, 0);
}

#[test]
fn inverted_line_needs_the_other_polarity() {
    let mut l = Line::new(30.0);
    l.active_high = true;
    l.idle(5);
    l.index_frame(0x42);
    l.index_frame(0x43);
    let (fs, _) = decode(&l.edges, false, l.t as u32 + 10_000);
    assert!(fs.is_empty(), "normal polarity must not decode an inverted line");
    let (fs, _) = decode(&l.edges, true, l.t as u32 + 10_000);
    assert_eq!(indices(&fs), vec![0x42, 0x43]);
}

#[test]
fn a_clock_is_not_a_frame() {
    // A square wave (a clock on the net, or the MCU's own PCK) never decodes.
    let mut edges = vec![];
    for i in 0..10_000u32 {
        edges.push((i * 17, i % 2 == 0));
    }
    let (fs, _) = decode(&edges, false, 200_000);
    assert!(fs.is_empty());
    let (fs, _) = decode(&edges, true, 200_000);
    assert!(fs.is_empty());
}

#[test]
fn random_noise_never_produces_a_frame() {
    let mut rng = Rng(0xdead_beef);
    let mut edges = vec![];
    let mut t = 0u32;
    let mut h = true;
    for _ in 0..200_000 {
        t = t.wrapping_add(1 + rng.next() % 400);
        h = !h;
        edges.push((t, h));
    }
    let (fs, _) = decode(&edges, false, t.wrapping_add(10_000));
    // A random pulse train can in principle spell a valid frame; the
    // complement byte makes it unlikely enough that none appears here.
    assert!(fs.is_empty(), "noise decoded as {:?}", indices(&fs));
}

#[test]
fn glitch_drops_the_frame_and_the_next_one_still_decodes() {
    let mut l = Line::new(50.0);
    l.idle(5);
    l.index_frame(0x11);
    let cut = l.edges.len();
    l.index_frame(0x22);
    l.index_frame(0x33);
    // A 2-unit glitch in the middle of 0x22.
    let mid = cut + 10;
    let (t, h) = l.edges[mid];
    l.edges.insert(mid + 1, (t + 2, !h));
    l.edges.insert(mid + 2, (t + 4, h));
    let (fs, dropped) = decode(&l.edges, false, l.t as u32 + 10_000);
    assert_eq!(indices(&fs), vec![0x11, 0x33]);
    assert!(dropped >= 1);
}

#[test]
fn time_counter_wraps() {
    let mut l = Line::new(1000.0);
    l.t = u32::MAX as u64 - 30_000; // the frame straddles the wrap
    l.idle(1);
    l.index_frame(0x99);
    let edges: Vec<(u32, bool)> = l.edges.iter().map(|&(t, h)| (t, h)).collect();
    let end = (l.t as u64 + 10_000) as u32;
    let (fs, _) = decode(&edges, false, end);
    assert_eq!(indices(&fs), vec![0x99]);
}

#[test]
fn truncated_frame_is_dropped_not_misread() {
    let mut l = Line::new(25.0);
    l.idle(5);
    l.index_frame(0x5a);
    // Cut the second frame after a few bits by going idle for good.
    let keep = l.edges.len();
    l.index_frame(0x77);
    l.edges.truncate(keep + 9);
    let last = l.edges.last().unwrap().0;
    if !l.edges.last().unwrap().1 {
        l.edges.push((last + 25, true));
    }
    let (fs, dropped) = decode(&l.edges, false, last + 100_000);
    assert_eq!(indices(&fs), vec![0x5a]);
    assert_eq!(dropped, 1);
}

#[test]
fn clock_section_round_trip() {
    let counts = [0u16, 96, 1234, 0xffff, 7, 512, 0, 0, 1, 2, 3];
    let mut b = [0u8; MAX_FRAME_BYTES];
    let n = frame_bytes(0x80, Some((10, 6, &counts)), &mut b);
    assert_eq!(n, 4 + 2 * counts.len());
    let mut l = Line::new(60.0);
    l.idle(3);
    l.frame(&b[..n]);
    let (fs, _) = decode(&l.edges, false, l.t as u32 + 100_000);
    assert_eq!(fs.len(), 1);
    let f = fs[0];
    assert_eq!(f.index(), 0x80);
    let c = f.clocks().unwrap();
    assert_eq!((c.div_bits, c.win_bits, c.n), (10, 6, counts.len() as u8));
    assert_eq!(&c.counts[..counts.len()], &counts);
}

#[test]
fn bad_check_bytes_are_rejected() {
    assert!(Frame::from_bytes(&[5, !5], 1).is_some());
    assert!(Frame::from_bytes(&[5, 5], 1).is_none());
    assert!(Frame::from_bytes(&[5], 1).is_none());
    assert!(Frame::from_bytes(&[5, !5, 0xa6], 1).is_none()); // odd length
    let mut b = [0u8; MAX_FRAME_BYTES];
    let n = frame_bytes(3, Some((10, 6, &[100, 200])), &mut b);
    assert!(Frame::from_bytes(&b[..n], 1).is_some());
    b[4] ^= 1;
    assert!(Frame::from_bytes(&b[..n], 1).is_none(), "bad csum accepted");
}

#[test]
fn clock_hz_examples() {
    // 48 MHz into a 10-bit counter: the top bit toggles every 512 cycles,
    // 93,750 times a second. Window 64 ticks of 16 us (1,536 CPU cycles at
    // 96 MHz) = 1.024 ms -> 96 toggles.
    assert_eq!(clock_hz(96, 1536, 96_000_000, 10, 6), 48_000_000);
    // 11.2896 MHz in the same window: 22.58 toggles -> 22 counted.
    let hz = clock_hz(22, 1536, 96_000_000, 10, 6);
    assert!((10_500_000..=11_300_000).contains(&hz), "{hz}");
    assert_eq!(clock_hz(0, 1536, 96_000_000, 10, 6), 0);
    assert_eq!(clock_hz(10, 0, 96_000_000, 10, 6), 0);
}

#[test]
fn tally_keeps_distinct_indices() {
    let mut t = Tally::new();
    assert!(t.add(7));
    assert!(!t.add(7));
    for _ in 0..8 {
        t.add(7);
    }
    assert!(t.add(9));
    assert_eq!(t.hits(), &[(7, 10), (9, 1)]);
    assert_eq!(t.dominant(80), Some(7));
    assert_eq!(t.dominant(95), None);
    t.add(1);
    t.add(2);
    t.add(3); // fifth distinct index
    assert_eq!(t.overflow, 1);
}
