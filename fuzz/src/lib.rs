use nodelite_proto::{BrowserMessage, WireMessage};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub fn fuzz_wire_message(data: &[u8]) {
    let _ = serde_json::from_slice::<WireMessage>(data);
}

pub fn fuzz_browser_message(data: &[u8]) {
    let _ = serde_json::from_slice::<BrowserMessage>(data);
}

pub fn fuzz_protocol_messages(data: &[u8]) {
    fuzz_wire_message(data);
    fuzz_browser_message(data);
}

pub fn run_target_from_args(target: fn(&[u8])) -> io::Result<()> {
    let mut args = std::env::args_os().skip(1).peekable();
    if args.peek().is_none() {
        let mut data = Vec::new();
        io::stdin().read_to_end(&mut data)?;
        target(&data);
        return Ok(());
    }

    for path in args {
        run_path(Path::new(&path), target)?;
    }
    Ok(())
}

pub fn run_fixed_iteration_smoke(iterations: usize) {
    let mut state = 0x4e4f44454c495445_u64;
    let seeds: [&[u8]; 12] = [
        b"",
        b"null",
        b"{",
        b"[]",
        b"{\"type\":\"ping\"}",
        b"{\"type\":\"ping\",\"nonce\":7}",
        b"{\"type\":\"pong\"}",
        b"{\"type\":\"server_notice\",\"level\":\"info\",\"message\":\"authenticated\"}",
        b"{\"type\":\"node_removed\",\"generated_at\":\"2026-05-31T12:00:00Z\",\"node_id\":\"hk-01\"}",
        b"{\"type\":42}",
        b"{\"type\":\"metrics\",\"snapshot\":{}}",
        b"{\"type\":\"metrics\",\"snapshot\":{\"disks\":[[[[[[[[[]]]]]]]]}}}",
    ];

    for iteration in 0..iterations {
        let seed = seeds[iteration % seeds.len()];
        let len = next_random(&mut state) as usize % 512;
        let mut data = Vec::with_capacity(seed.len() + len);
        data.extend_from_slice(seed);
        for index in 0..len {
            let byte = (next_random(&mut state) >> ((index % 8) * 8)) as u8;
            if index % 5 == 0 && !data.is_empty() {
                let offset = next_random(&mut state) as usize % data.len();
                data[offset] = byte;
            } else {
                data.push(byte);
            }
        }
        fuzz_protocol_messages(&data);
    }
}

fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 7;
    *state ^= *state >> 9;
    *state ^= *state << 8;
    *state
}

fn run_path(path: &Path, target: fn(&[u8])) -> io::Result<()> {
    if path.is_dir() {
        let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            run_path(&entry.path(), target)?;
        }
        return Ok(());
    }

    let data = fs::read(path)?;
    target(&data);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_WIRE_MESSAGES: &[&[u8]] = &[
        br#"{"type":"ping","nonce":7}"#,
        br#"{"type":"pong","nonce":7}"#,
        br#"{"type":"server_notice","level":"info","message":"authenticated"}"#,
        br#"{"type":"refresh_token_request"}"#,
        br#"{"type":"agent_logs","entries":[{"occurred_at":"2026-05-07T01:02:03Z","level":"warn","message":"retrying"}]}"#,
    ];

    const VALID_BROWSER_MESSAGES: &[&[u8]] = &[
        br#"{"type":"ping"}"#,
        br#"{"type":"pong"}"#,
        br#"{"type":"node_removed","generated_at":"2026-05-31T12:00:00Z","node_id":"hk-01"}"#,
    ];

    #[test]
    fn valid_wire_message_fixtures_round_trip() {
        for fixture in VALID_WIRE_MESSAGES {
            let message: WireMessage =
                serde_json::from_slice(fixture).expect("wire fixture should parse");
            let encoded = serde_json::to_vec(&message).expect("wire fixture should encode");
            fuzz_wire_message(&encoded);
        }
    }

    #[test]
    fn valid_browser_message_fixtures_round_trip() {
        for fixture in VALID_BROWSER_MESSAGES {
            let message: BrowserMessage =
                serde_json::from_slice(fixture).expect("browser fixture should parse");
            let encoded = serde_json::to_vec(&message).expect("browser fixture should encode");
            fuzz_browser_message(&encoded);
        }
    }

    #[test]
    fn malformed_inputs_do_not_panic() {
        for data in [
            b"" as &[u8],
            b"null",
            b"{",
            b"[]",
            b"{\"type\":42}",
            b"{\"type\":\"metrics\",\"snapshot\":{\"disks\":[[[[[[[[[]]]]]]]]}}}",
        ] {
            fuzz_protocol_messages(data);
        }
    }

    #[test]
    fn fixed_iteration_smoke_does_not_panic() {
        run_fixed_iteration_smoke(1024);
    }
}
