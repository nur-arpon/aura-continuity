//! Named pipe server (tokio). One line in, one line out, then the pipe instance is closed.
//! Every connection is answered by connect + `Config::deadline` whatever the client or the decider
//! does: a client that sends nothing gets `timeout`, an oversized line gets `malformed`.

use crate::engine::Engine;
use crate::proto::{self, reason, MAX_REQUEST};
use crate::sec::SecAttrs;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use tokio::sync::Semaphore;

/// Concurrent connections served; more get `busy` at once.
pub const MAX_CONNECTIONS: usize = 16;

/// `\\.\pipe\ContinuityUnlock.<SID>` (one per user, so fast user switching cannot collide)
pub fn pipe_name(sid: &str, test: bool) -> String {
    if test {
        format!(r"\\.\pipe\ContinuityUnlock-test.{sid}")
    } else {
        format!(r"\\.\pipe\ContinuityUnlock.{sid}")
    }
}

fn create(name: &str, sa: &SecAttrs, first: bool) -> std::io::Result<NamedPipeServer> {
    let mut o = ServerOptions::new();
    // first instance fails if anyone else already owns the name (no pipe squatting / no 2nd helper)
    o.first_pipe_instance(first).reject_remote_clients(true).pipe_mode(PipeMode::Byte).in_buffer_size(512).out_buffer_size(512);
    let mut raw = sa.sa;
    unsafe { o.create_with_security_attributes_raw(name, &mut raw as *mut _ as *mut std::ffi::c_void) }
}

/// Bind the pipe (fails if the name is taken) and return a future that serves forever.
pub fn bind(engine: Arc<Engine>, name: String, sddl: &str, log: Arc<dyn Fn(&str) + Send + Sync>) -> Result<impl std::future::Future<Output = ()>, String> {
    let sa = SecAttrs::from_sddl(sddl)?;
    let first = create(&name, &sa, true).map_err(|e| format!("create pipe {name}: {e} (another helper running?)"))?;
    Ok(serve(engine, name, sa, first, log))
}

async fn serve(engine: Arc<Engine>, name: String, sa: SecAttrs, mut server: NamedPipeServer, log: Arc<dyn Fn(&str) + Send + Sync>) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        if let Err(e) = server.connect().await {
            log(&format!("pipe connect: {e}"));
        } else {
            let conn = server;
            let deadline = Instant::now() + engine.cfg.deadline;
            let e = engine.clone();
            match slots.clone().try_acquire_owned() {
                Ok(permit) => {
                    tokio::spawn(async move {
                        handle(&e, conn, deadline).await;
                        drop(permit);
                    });
                }
                Err(_) => {
                    tokio::spawn(async move { reply(conn, &proto::not_approved_line("-", reason::BUSY)).await });
                }
            }
        }
        // next instance; on failure keep trying, never exit
        server = loop {
            match create(&name, &sa, false) {
                Ok(s) => break s,
                Err(e) => {
                    log(&format!("create pipe instance: {e}"));
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        };
    }
}

async fn read_line(pipe: &mut NamedPipeServer) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(64);
    let mut chunk = [0u8; 64];
    loop {
        let n = pipe.read(&mut chunk).await?;
        if n == 0 {
            return Ok(buf); // client closed its write side: judge what we have
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.iter().position(|b| *b == b'\n') {
            buf.truncate(i + 1);
            return Ok(buf);
        }
        if buf.len() > MAX_REQUEST {
            return Ok(buf); // parse_request rejects it as malformed
        }
    }
}

async fn handle(engine: &Engine, mut pipe: NamedPipeServer, deadline: Instant) {
    let line = match tokio::time::timeout_at(deadline.into(), read_line(&mut pipe)).await {
        Err(_) => proto::not_approved_line("-", reason::TIMEOUT),
        Ok(Err(_)) => return, // client went away
        Ok(Ok(l)) => engine.answer(&l, deadline).await,
    };
    reply(pipe, &line).await;
}

async fn reply(mut pipe: NamedPipeServer, line: &str) {
    let _ = tokio::time::timeout(Duration::from_millis(150), async {
        pipe.write_all(line.as_bytes()).await?;
        pipe.flush().await
    })
    .await;
    // Disconnecting discards unread data, so wait (bounded) for the client to read and close its
    // end; the instance is then just dropped.
    let mut b = [0u8; 16];
    let _ = tokio::time::timeout(Duration::from_millis(300), async {
        while let Ok(n) = pipe.read(&mut b).await {
            if n == 0 {
                break;
            }
        }
    })
    .await;
}
