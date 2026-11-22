//! Cross-check against the FPGA side: decode the edges that the Amaranth
//! simulation of the pin-finder core produced (`fpga/pinfind/test_pinfind.py
//! --write-golden` -> `fpga/pinfind/testdata/sim_edges.txt`).
//!
//! That run: 3 balls, 2 frames per ball, 2 loops, tick = 4 clocks, 50 MHz
//! clock (time unit = one 20 ns clock), probes at 12.5 MHz and 3.125 MHz with
//! div_bits 4 and win_bits 4.

use pinfind::{Decoder, Frame};

fn load() -> Vec<Vec<(u32, bool)>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fpga/pinfind/testdata/sim_edges.txt");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut balls: Vec<Vec<(u32, bool)>> = vec![];
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let w: Vec<u32> = line.split_whitespace().map(|x| x.parse().unwrap()).collect();
        let (k, t, lv) = (w[0] as usize, w[1], w[2] != 0);
        if balls.len() <= k {
            balls.resize(k + 1, vec![]);
        }
        balls[k].push((t, lv));
    }
    balls
}

fn decode(edges: &[(u32, bool)]) -> Vec<Frame> {
    let mut d = Decoder::new(false);
    // The line idles high before the first edge.
    d.edge(0, true);
    let mut out = vec![];
    for &(t, h) in edges {
        if let Some(f) = d.poll(t) {
            out.push(f);
        }
        if let Some(f) = d.edge(t, h) {
            out.push(f);
        }
    }
    if let Some(f) = d.poll(u32::MAX / 2) {
        out.push(f);
    }
    assert_eq!(d.dropped, 0, "dropped frames");
    out
}

#[test]
fn amaranth_frames_decode() {
    let balls = load();
    assert_eq!(balls.len(), 3);
    for (k, edges) in balls.iter().enumerate() {
        let frames = decode(edges);
        assert_eq!(frames.len(), 4, "ball {k}: 2 frames x 2 loops");
        for f in &frames {
            assert_eq!(f.index() as usize, k);
            assert_eq!(f.unit, 4, "tick = 4 clocks exactly in simulation");
            let c = f.clocks().expect("clock section");
            assert_eq!((c.div_bits, c.win_bits, c.n), (4, 4, 2));
        }
        // By the second loop the windows are full: 12.5 MHz reads back as such.
        let last = frames.last().unwrap();
        let c = last.clocks().unwrap();
        let hz0 = c.hz(0, last.unit, 50_000_000);
        assert!((6_250_000..=18_750_000).contains(&hz0), "probe 0: {hz0} Hz");
        let hz1 = c.hz(1, last.unit, 50_000_000);
        assert!(hz1 <= 6_250_000, "probe 1: {hz1} Hz");
    }
}
