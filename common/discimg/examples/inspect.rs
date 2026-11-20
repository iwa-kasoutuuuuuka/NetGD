//! Print the track table NetGD's own parsers make of a GDI or CDI file.
//!
//!     cargo run --example inspect -- path/to/disc.gdi
//!     cargo run --example inspect -- path/to/disc.cdi
//!
//! One line per track, machine readable (tools/mksdcard/verify.py compares it with the
//! manifest): `track N lba=.. ctrl=.. ssize=.. count=.. offset=.. file=..`.
//! GDI: `count` comes from the size of the track file; `offset` is 0.
//! CDI: `count` and `offset` are what the descriptor says (offset is past the pregap).

use discimg::{cdi, gdi};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: inspect <file.gdi|file.cdi>");
            std::process::exit(2);
        }
    };
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".gdi") {
        let data = std::fs::read(&path).expect("read gdi");
        let tracks = match gdi::parse_vec(&data) {
            Ok(t) => t,
            Err(e) => {
                println!("gdi parse error: {:?}", e);
                std::process::exit(1);
            }
        };
        let dir = std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new("."));
        println!("format=gdi tracks={}", tracks.len());
        for t in tracks.iter() {
            let size = std::fs::metadata(dir.join(t.filename.as_str())).map(|m| m.len()).unwrap_or(0);
            let count = size / t.sector_size as u64;
            println!(
                "track {} lba={} ctrl={} ssize={} count={} offset={} file={}",
                t.number, t.start_lba, t.ctrl, t.sector_size, count, t.file_offset, t.filename
            );
        }
    } else if lower.ends_with(".cdi") {
        let mut f = File::open(&path).expect("open cdi");
        let len = f.metadata().unwrap().len() as u32;
        let mut sink = discimg::VecSink::new();
        let r = cdi::parse(
            len,
            |off, buf| {
                f.seek(SeekFrom::Start(off as u64)).map_err(|_| ())?;
                f.read_exact(buf).map_err(|_| ())
            },
            &mut sink,
        );
        match r {
            Ok(sessions) => {
                println!("format=cdi sessions={} tracks={}", sessions, sink.0.len());
                for t in sink.0.iter() {
                    println!(
                        "track {} lba={} ctrl={} ssize={} count={} offset={} file=-",
                        t.number, t.start_lba, t.ctrl, t.sector_size, t.sector_count, t.file_offset
                    );
                }
            }
            Err(e) => {
                println!("cdi parse error: {:?}", e);
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("unknown extension (want .gdi or .cdi)");
        std::process::exit(2);
    }
}
