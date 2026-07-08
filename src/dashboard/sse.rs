use std::io::{BufRead, BufReader, Write};
use std::process::Child;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use tiny_http::{Request, Response};

use super::AppState;
use crate::{docker, state};

/// Raw response head for the log stream. We take over the socket with
/// `into_writer` (rather than a `Response`) because tiny_http only flushes a
/// `Response` body once, after the body reader returns — which never happens
/// for an endless stream, so frames would sit in its buffer. Owning the writer
/// lets us flush after every event. `Connection: close` means the stream is
/// terminated by the socket closing and the browser won't reuse it.
const SSE_HEAD: &str = "HTTP/1.1 200 OK\r\n\
Content-Type: text/event-stream\r\n\
Cache-Control: no-cache\r\n\
X-Accel-Buffering: no\r\n\
Connection: close\r\n\
\r\n";

/// How long to wait for a log line before emitting a heartbeat. The heartbeat
/// both keeps proxies from idling the connection out and, more importantly,
/// forces a socket write on an idle stream so a client disconnect surfaces as
/// a write error within this bound instead of hanging forever.
const HEARTBEAT: Duration = Duration::from_secs(15);

/// Kills and reaps the log child on drop, so closing a log panel (or a handler
/// panic) never leaks a `docker compose logs -f` process.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Stream a berth's container logs to the client as Server-Sent Events until
/// the client disconnects or the containers stop.
pub fn stream_logs(request: Request, state: &AppState, name: &str) {
    let berth = match state::get(&state.root, name) {
        Ok(Some(berth)) => berth,
        Ok(None) => return respond_plain(request, 404, "no such berth"),
        Err(e) => return respond_plain(request, 500, &e),
    };

    let env_file = state.root.join(".berth").join(format!("{name}.env"));

    let mut child = match docker::logs_child(
        &state.root,
        &berth.compose_file,
        &env_file,
        &berth.compose_project,
    ) {
        Ok(child) => child,
        Err(e) => return respond_plain(request, 500, &e),
    };

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return respond_plain(request, 500, "could not capture log output");
        }
    };

    // Order matters for teardown: the child is now owned by the guard, whose
    // drop kills it. Killing closes the stdout pipe, which unblocks the reader
    // thread's `read_until`, letting the final `join` return without deadlock.
    let guard = ChildGuard(child);

    // A dedicated thread turns the blocking pipe into channel messages so the
    // main loop can wait with a timeout and heartbeat.
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            // Read raw bytes, not `lines()`: log output isn't guaranteed UTF-8,
            // and a decode error must not tear the whole stream down.
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(std::mem::take(&mut line)).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut writer = request.into_writer();

    if writer.write_all(SSE_HEAD.as_bytes()).is_ok() && writer.flush().is_ok() {
        loop {
            let write = match rx.recv_timeout(HEARTBEAT) {
                Ok(line) => writer.write_all(sse_data_frame(&line).as_bytes()),
                Err(RecvTimeoutError::Timeout) => writer.write_all(b": ping\n\n"),
                Err(RecvTimeoutError::Disconnected) => {
                    // Containers stopped: tell the client to close so its
                    // EventSource doesn't reconnect and respawn `logs -f`.
                    let _ = writer.write_all(b"event: end\ndata: stream ended\n\n");
                    let _ = writer.flush();
                    break;
                }
            };

            if write.and_then(|()| writer.flush()).is_err() {
                break; // client gone
            }
        }
    }

    drop(guard); // kill + reap the child, unblocking the reader
    let _ = reader.join();
}

/// Format one log line as an SSE `data:` event, splitting embedded newlines
/// into multiple `data:` lines per the SSE spec and decoding lossily.
fn sse_data_frame(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut frame = String::with_capacity(text.len() + 8);
    for line in text.trim_end_matches(['\r', '\n']).split('\n') {
        frame.push_str("data: ");
        frame.push_str(line.trim_end_matches('\r'));
        frame.push('\n');
    }
    frame.push('\n');
    frame
}

fn respond_plain(request: Request, code: u16, message: &str) {
    let _ = request.respond(Response::from_string(message).with_status_code(code));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_frame() {
        assert_eq!(sse_data_frame(b"hello\n"), "data: hello\n\n");
        assert_eq!(sse_data_frame(b"hello"), "data: hello\n\n");
    }

    #[test]
    fn multi_line_frame_splits_per_spec() {
        assert_eq!(sse_data_frame(b"a\nb\n"), "data: a\ndata: b\n\n");
    }

    #[test]
    fn invalid_utf8_is_lossy_not_fatal() {
        let frame = sse_data_frame(&[0xff, 0xfe, b'x', b'\n']);
        assert!(frame.starts_with("data: "));
        assert!(frame.ends_with("\n\n"));
    }
}
