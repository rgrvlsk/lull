//! Native-messaging framing and transparent bridge to the daemon (contract §9).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;

use crate::browsers::{CHROMIUM_EXTENSION_ID, FIREFOX_ADDON_ID};
use crate::paths::Paths;

const MAX_MESSAGE: usize = 1024 * 1024;

pub fn run(args: Vec<String>) -> ExitCode {
    let chromium_origin = format!("chrome-extension://{CHROMIUM_EXTENSION_ID}/");
    let allowed = args.first() == Some(&chromium_origin)
        || args.get(1).is_some_and(|id| id == FIREFOX_ADDON_ID);
    if !allowed {
        eprintln!("lull-nm-host: caller is not allowed");
        return ExitCode::FAILURE;
    }

    let Ok(socket) = UnixStream::connect(Paths::from_env().socket()) else {
        eprintln!("lull-nm-host: cannot connect to the daemon");
        return ExitCode::FAILURE;
    };
    let (Ok(mut writer), Ok(reader)) = (socket.try_clone(), socket.try_clone()) else {
        eprintln!("lull-nm-host: cannot prepare the daemon connection");
        return ExitCode::FAILURE;
    };

    let (done, finished) = mpsc::channel();
    let input_done = done.clone();
    thread::spawn(move || {
        let result = browser_to_daemon(&mut io::stdin().lock(), &mut writer);
        let _ = input_done.send(result);
    });
    thread::spawn(move || {
        let result = daemon_to_browser(&mut BufReader::new(reader), &mut io::stdout());
        let _ = done.send(result);
    });

    let result = finished.recv();
    let _ = socket.shutdown(Shutdown::Both);
    // ponytail: workers live only in this host process; returning from main ends a
    // blocked stdio worker. If embedded later, use interruptible fd reads and joins.
    match result {
        Ok(Ok(())) => ExitCode::SUCCESS,
        _ => {
            eprintln!("lull-nm-host: invalid message or bridge I/O failure");
            ExitCode::FAILURE
        }
    }
}

fn invalid_message() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid native message")
}

fn read_frame(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut prefix = [0; 4];
    loop {
        match reader.read(&mut prefix[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    reader.read_exact(&mut prefix[1..])?;
    let length = u32::from_ne_bytes(prefix) as usize;
    if length > MAX_MESSAGE {
        return Err(invalid_message());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let value: serde_json::Value = serde_json::from_slice(&body).map_err(|_| invalid_message())?;
    let compact = serde_json::to_vec(&value).map_err(|_| invalid_message())?;
    if compact.len() > MAX_MESSAGE {
        return Err(invalid_message());
    }
    Ok(Some(compact))
}

fn write_frame(writer: &mut impl Write, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_MESSAGE {
        return Err(invalid_message());
    }
    writer.write_all(&(body.len() as u32).to_ne_bytes())?;
    writer.write_all(body)?;
    writer.flush()
}

fn browser_to_daemon(reader: &mut impl Read, writer: &mut impl Write) -> io::Result<()> {
    while let Some(body) = read_frame(reader)? {
        writer.write_all(&body)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}

fn daemon_to_browser(reader: &mut impl BufRead, writer: &mut impl Write) -> io::Result<()> {
    let mut body = Vec::new();
    let mut oversized = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            if !oversized && !body.is_empty() {
                write_frame(writer, &body)?;
            }
            return Ok(());
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let length = newline.unwrap_or(chunk.len());
        if !oversized {
            if body.len() + length > MAX_MESSAGE {
                eprintln!("lull-nm-host: dropped oversized daemon message");
                body.clear();
                oversized = true;
            } else {
                body.extend_from_slice(&chunk[..length]);
            }
        }
        reader.consume(length + usize::from(newline.is_some()));
        if newline.is_some() {
            if !oversized {
                write_frame(writer, &body)?;
            }
            body.clear();
            oversized = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct SplitRead<'a> {
        bytes: &'a [u8],
        seed: u64,
    }

    fn random(seed: &mut u64) -> u64 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        *seed
    }

    impl Read for SplitRead<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let count = buf.len().min((random(&mut self.seed) % 31 + 1) as usize);
            self.bytes.read(&mut buf[..count])
        }
    }

    #[test]
    fn randomized_frames_round_trip_across_read_boundaries() {
        for seed in 0..128 {
            let mut state = seed;
            let mut values = Vec::new();
            let mut wire = Vec::new();
            for _ in 0..16 {
                let text: String = (0..random(&mut state) % 300)
                    .map(|_| match random(&mut state) % 7 {
                        0 => '\n',
                        1 => '"',
                        2 => '\\',
                        3 => 'é',
                        4 => '🦀',
                        _ => (b'a' + (random(&mut state) % 26) as u8) as char,
                    })
                    .collect();
                let value = json!({"id": random(&mut state), "text": text});
                write_frame(&mut wire, &serde_json::to_vec_pretty(&value).unwrap()).unwrap();
                values.push(value);
            }
            let mut reader = SplitRead { bytes: &wire, seed };
            for expected in values {
                let body = read_frame(&mut reader).unwrap().unwrap();
                assert!(!body.contains(&b'\n'));
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                    expected
                );
            }
            assert!(read_frame(&mut reader).unwrap().is_none());
        }
    }

    #[test]
    fn rejects_oversized_truncated_and_malformed_frames() {
        for length in [MAX_MESSAGE as u32 + 1, u32::MAX] {
            assert!(read_frame(&mut length.to_ne_bytes().as_slice()).is_err());
        }
        for count in 1..4 {
            assert!(read_frame(&mut &3_u32.to_ne_bytes()[..count]).is_err());
        }
        for body in [b"null".as_slice(), b"\"secret\"", b"[1,2,3]"] {
            let mut wire = Vec::new();
            write_frame(&mut wire, body).unwrap();
            for count in 4..wire.len() {
                assert!(read_frame(&mut &wire[..count]).is_err());
            }
        }
        for body in [b"".as_slice(), b"not json", b"{}{}", b"\"\xff\""] {
            let mut wire = Vec::new();
            write_frame(&mut wire, body).unwrap();
            assert!(read_frame(&mut wire.as_slice()).is_err());
        }
        let mut output = Vec::new();
        assert!(write_frame(&mut output, &vec![b' '; MAX_MESSAGE + 1]).is_err());
        assert!(output.is_empty());
    }
}
