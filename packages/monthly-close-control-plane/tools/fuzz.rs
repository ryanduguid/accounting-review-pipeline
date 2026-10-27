//! Replay fabricated corpus bytes, then apply bounded deterministic mutations.
use monthly_close_control_plane::fuzzing::exercise;
use std::{
    env, fs,
    process::ExitCode,
    time::{Duration, Instant},
};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: close-control-fuzz CORPUS SECONDS (0..600)".into());
    }
    let seconds: u64 = args[1].to_str().ok_or("invalid seconds")?.parse()?;
    if seconds > 600 {
        return Err("seconds must be at most 600".into());
    }
    let mut paths: Vec<_> = fs::read_dir(&args[0])?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    paths.sort();
    let mut seeds = Vec::new();
    for path in paths {
        if !path.is_file() {
            return Err("corpus entries must be files".into());
        }
        let bytes = fs::read(path)?;
        if bytes.len() > 4096 {
            return Err("corpus seed exceeds 4096 bytes".into());
        }
        exercise(&bytes);
        seeds.push(bytes);
    }
    if seeds.is_empty() {
        return Err("corpus is empty".into());
    }
    let start = Instant::now();
    let mut state = 1u64;
    let mut count = 0usize;
    while start.elapsed() < Duration::from_secs(seconds) {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut bytes = seeds[count % seeds.len()].clone();
        let offset = (state >> 32) as usize % (bytes.len() + 1);
        if offset < bytes.len() {
            match state % 3 {
                0 => {
                    bytes.remove(offset);
                }
                1 => bytes[offset] ^= 1 << (state % 8),
                _ => bytes[offset] = (state >> 24) as u8,
            }
        } else if bytes.len() < 4096 {
            bytes.push((state >> 24) as u8);
        }
        exercise(&bytes);
        count += 1;
    }
    println!(
        "replayed {} seeds; {count} mutations; no panic",
        seeds.len()
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("close-control-fuzz: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_bytes_are_refused_without_panicking() {
        for bytes in [vec![], vec![0xff, 0x00], b"a,b\n\"unterminated".to_vec()] {
            exercise(&bytes);
        }
    }
}
