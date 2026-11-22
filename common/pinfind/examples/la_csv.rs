//! Decode pin-finder frames from a logic-analyser capture (the GD-ROM
//! connector side, which the MCU cannot see).
//!
//!     cargo run --example la_csv -- capture.csv [--rate HZ] [--skip-cols N]
//!
//! Input: CSV as written by `sigrok-cli ... -O csv` (or PulseView's CSV
//! export): `;` comment lines, one header line with the channel names, then
//! one row of 0/1 per sample. `--skip-cols N` drops N leading columns (a time
//! column, for example). Time is the sample number, so the sample rate only
//! matters for `--rate`, which turns clock-probe counts into Hz.
//!
//! Output, one line per channel and index (the format `tools/pinfind/pins2asf.py`
//! reads with `--la`):
//!
//!     PF ch=D3 idx=37 n=12
//!
//! plus `PF act ch=.. edges=..` for channels that toggled without a frame,
//! and `PF clk ...` for the clock-probe section if the frames carry one.

use pinfind::{Decoder, Frame, Tally};
use std::io::{BufRead, BufReader};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut rate: Option<u32> = None;
    let mut skip = 0usize;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rate" => rate = Some(args.next().expect("--rate HZ").parse().expect("--rate HZ")),
            "--skip-cols" => skip = args.next().expect("--skip-cols N").parse().expect("--skip-cols N"),
            "-h" | "--help" => {
                eprintln!("usage: la_csv FILE.csv [--rate HZ] [--skip-cols N]");
                return;
            }
            _ => path = Some(a),
        }
    }
    let path = path.expect("usage: la_csv FILE.csv [--rate HZ] [--skip-cols N]");
    let f = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let report = decode(BufReader::new(f), skip);
    print!("{}", report.render(rate));
}

struct Channel {
    name: String,
    // Both polarities: a level shifter or inverter on the probe path flips it.
    dec: [Decoder; 2],
    tally: Tally,
    edges: u64,
    level: Option<bool>,
}

struct Report {
    chans: Vec<Channel>,
    last_clock: Option<Frame>,
}

fn decode(r: impl BufRead, skip: usize) -> Report {
    let mut chans: Vec<Channel> = vec![];
    let mut last_clock = None;
    let mut sample: u32 = 0;
    for line in r.lines() {
        let line = line.expect("read error");
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(|s| s.trim()).skip(skip).collect();
        let numeric = fields.iter().all(|f| *f == "0" || *f == "1");
        if chans.is_empty() {
            let names: Vec<String> = if numeric {
                (0..fields.len()).map(|i| format!("{i}")).collect()
            } else {
                fields.iter().map(|s| s.to_string()).collect()
            };
            chans = names
                .into_iter()
                .map(|name| Channel {
                    name,
                    dec: [Decoder::new(false), Decoder::new(true)],
                    tally: Tally::new(),
                    edges: 0,
                    level: None,
                })
                .collect();
            if !numeric {
                continue;
            }
        }
        if !numeric {
            continue;
        }
        for (c, f) in chans.iter_mut().zip(fields.iter()) {
            let high = *f == "1";
            if c.level != Some(high) {
                if c.level.is_some() {
                    c.edges += 1;
                }
                c.level = Some(high);
                for d in &mut c.dec {
                    if let Some(fr) = d.edge(sample, high) {
                        c.tally.add(fr.index());
                        if fr.clocks().is_some() {
                            last_clock = Some(fr);
                        }
                    }
                }
            }
            if sample % 64 == 0 {
                for d in &mut c.dec {
                    if let Some(fr) = d.poll(sample) {
                        c.tally.add(fr.index());
                        if fr.clocks().is_some() {
                            last_clock = Some(fr);
                        }
                    }
                }
            }
        }
        sample = sample.wrapping_add(1);
    }
    // Close frames that end with the capture.
    for c in &mut chans {
        for d in &mut c.dec {
            if let Some(fr) = d.poll(sample.wrapping_add(1 << 30)) {
                c.tally.add(fr.index());
            }
        }
    }
    Report { chans, last_clock }
}

impl Report {
    fn render(&self, rate: Option<u32>) -> String {
        let mut s = String::new();
        for c in &self.chans {
            for &(idx, n) in c.tally.hits() {
                s += &format!("PF ch={} idx={} n={}\n", c.name, idx, n);
            }
            if c.tally.hits().is_empty() && c.edges > 0 {
                s += &format!("PF act ch={} edges={}\n", c.name, c.edges);
            }
        }
        if let Some(f) = self.last_clock {
            let r = f.clocks().unwrap();
            for i in 0..r.n as usize {
                let hz = rate.map(|t| r.hz(i, f.unit, t));
                s += &format!(
                    "PF clk c={} cnt={} unit={} div={} win={}{}\n",
                    i,
                    r.counts[i],
                    f.unit,
                    r.div_bits,
                    r.win_bits,
                    hz.map(|h| format!(" hz={h}")).unwrap_or_default()
                );
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_with_header_and_comments() {
        // Channel D1 carries ball 5 at 4 samples per tick, D0 stays high.
        let mut pulses = vec![];
        let mut b = [0u8; pinfind::MAX_FRAME_BYTES];
        let n = pinfind::frame_bytes(5, None, &mut b);
        pinfind::encode(&b[..n], |a, t| pulses.push((a, t)));
        let mut csv = String::from("; sigrok-cli\n; samplerate 1 MHz\nD0,D1\n");
        for _ in 0..10 {
            csv += "1,1\n";
        }
        for (active, ticks) in pulses {
            for _ in 0..ticks * 4 {
                csv += if active { "1,0\n" } else { "1,1\n" };
            }
        }
        for _ in 0..40 {
            csv += "1,1\n";
        }
        let r = decode(csv.as_bytes(), 0);
        let out = r.render(None);
        assert_eq!(out, "PF ch=D1 idx=5 n=1\n");
    }
}
