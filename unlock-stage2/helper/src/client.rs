//! Minimal blocking pipe client (pipe-test.exe and the integration tests). The C++ tile does the
//! same with CreateFileW + overlapped ReadFile; see HELPER-CONTRACT.md.

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Send `request` (bytes, sent as-is) and return the answer line, or an error by `timeout`.
/// The work runs on its own thread so a stuck pipe can never hold the caller past `timeout`.
pub fn ask_raw(pipe: &str, request: &[u8], timeout: Duration) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    let pipe = pipe.to_string();
    let req = request.to_vec();
    std::thread::spawn(move || {
        let _ = tx.send(exchange(&pipe, &req, Instant::now() + timeout));
    });
    rx.recv_timeout(timeout).unwrap_or_else(|_| Err(format!("no answer within {} ms", timeout.as_millis())))
}

fn exchange(pipe: &str, req: &[u8], deadline: Instant) -> Result<String, String> {
    let mut f = loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(pipe) {
            Ok(f) => break f,
            // 231 = ERROR_PIPE_BUSY: every instance is in use, try again shortly
            Err(e) if e.raw_os_error() == Some(231) && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("open {pipe}: {e}")),
        }
    };
    if !req.is_empty() {
        f.write_all(req).map_err(|e| format!("write: {e}"))?;
    }
    let mut out = Vec::new();
    let mut b = [0u8; 256];
    loop {
        let n = f.read(&mut b).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&b[..n]);
        if out.contains(&b'\n') || out.len() > crate::proto::MAX_ANSWER {
            break;
        }
    }
    String::from_utf8(out).map_err(|_| "answer is not UTF-8".into())
}

/// Connect and send nothing (tests the server's timer for silent clients).
pub fn ask_silent(pipe: &str, timeout: Duration) -> Result<String, String> {
    ask_raw(pipe, b"", timeout)
}
