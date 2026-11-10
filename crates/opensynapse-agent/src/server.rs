//! Pipe server: one accept thread, one small thread per connection. Each
//! connection says Hello with its role; Control connections forward requests to
//! the core, Subscribe connections stream events. All I/O blocks; nothing polls.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

use opensynapse_ipc::codec::{decode_client, encode_server};
use opensynapse_ipc::frame::{read_frame, write_frame};
use opensynapse_ipc::msg::{
    ClientFrame, ErrorKind, Event, IpcError, PROTOCOL_VERSION, Reply, Role, ServerFrame,
};
use opensynapse_ipc::transport::Listener;

use crate::core::CoreMsg;

/// Stack of a connection thread: it only decodes frames and waits on channels.
const CONN_STACK: usize = 64 * 1024;
/// Give up accepting after this many errors in a row (avoids a hot loop).
const MAX_ACCEPT_ERRORS: u32 = 10;

/// Start the accept thread. It ends when `stop` is set and the listener's waker
/// unblocks it. `flushed` gets a message once a `ShuttingDown` reply is written.
pub fn spawn(
    mut listener: Listener,
    core: Sender<CoreMsg>,
    stop: Arc<AtomicBool>,
    flushed: Sender<()>,
) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("opensynapse-accept".into())
        .spawn(move || {
            let mut errors = 0;
            loop {
                let accepted = listener.accept();
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                match accepted {
                    Ok(conn) => {
                        errors = 0;
                        let core = core.clone();
                        let flushed = flushed.clone();
                        let spawned = std::thread::Builder::new()
                            .name("opensynapse-conn".into())
                            .stack_size(CONN_STACK)
                            .spawn(move || {
                                if let Err(e) = serve(conn, &core, &flushed) {
                                    tracing::debug!(error = %e, "connection ended");
                                }
                            });
                        if let Err(e) = spawned {
                            tracing::warn!(error = %e, "cannot start connection thread");
                        }
                    }
                    Err(e) => {
                        errors += 1;
                        tracing::warn!(error = %e, "accept failed");
                        if errors >= MAX_ACCEPT_ERRORS {
                            tracing::error!("too many accept errors; no longer accepting");
                            break;
                        }
                    }
                }
            }
        })
}

fn send(conn: &mut impl Write, f: &ServerFrame) -> io::Result<()> {
    write_frame(conn, &encode_server(f))
}

fn bad_request(message: impl Into<String>) -> ServerFrame {
    ServerFrame::Response {
        id: 0,
        result: Err(IpcError {
            kind: ErrorKind::BadRequest,
            message: message.into(),
        }),
    }
}

/// Serve one connection until the client closes it or the core stops.
pub fn serve<S: Read + Write>(
    mut conn: S,
    core: &Sender<CoreMsg>,
    flushed: &Sender<()>,
) -> io::Result<()> {
    let role = match decode_client(&read_frame(&mut conn)?) {
        Ok(ClientFrame::Hello { version, role }) if version == PROTOCOL_VERSION => role,
        Ok(ClientFrame::Hello { version, .. }) => {
            let f = ServerFrame::Response {
                id: 0,
                result: Err(IpcError {
                    kind: ErrorKind::Version,
                    message: format!(
                        "client speaks protocol {version}, agent speaks {PROTOCOL_VERSION}"
                    ),
                }),
            };
            return send(&mut conn, &f);
        }
        Ok(ClientFrame::Request { .. }) => {
            return send(&mut conn, &bad_request("expected Hello first"));
        }
        Err(e) => return send(&mut conn, &bad_request(format!("bad Hello: {e}"))),
    };
    send(
        &mut conn,
        &ServerFrame::Welcome {
            version: PROTOCOL_VERSION,
            agent_pid: std::process::id(),
        },
    )?;
    match role {
        Role::Control => control(conn, core, flushed),
        Role::Subscribe => subscribe(conn, core),
    }
}

fn control<S: Read + Write>(
    mut conn: S,
    core: &Sender<CoreMsg>,
    flushed: &Sender<()>,
) -> io::Result<()> {
    loop {
        let body = match read_frame(&mut conn) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let (id, req) = match decode_client(&body) {
            Ok(ClientFrame::Request { id, req }) => (id, req),
            Ok(ClientFrame::Hello { .. }) => {
                return send(&mut conn, &bad_request("Hello sent twice"));
            }
            Err(e) => return send(&mut conn, &bad_request(format!("bad request: {e}"))),
        };
        let (tx, rx) = channel();
        if core.send(CoreMsg::Request { req, reply: tx }).is_err() {
            return Ok(());
        }
        let Ok(result) = rx.recv() else {
            return Ok(());
        };
        let shutting_down = matches!(result, Ok(Reply::ShuttingDown));
        send(&mut conn, &ServerFrame::Response { id, result })?;
        if shutting_down {
            conn.flush()?;
            let _ = flushed.send(());
            return Ok(());
        }
    }
}

fn subscribe<S: Write>(mut conn: S, core: &Sender<CoreMsg>) -> io::Result<()> {
    let (tx, rx) = channel();
    if core.send(CoreMsg::Subscribe(tx)).is_err() {
        return Ok(());
    }
    // Ends when the core drops the sender (shutdown) or the client goes away
    // (the write fails; the core prunes the sender on its next broadcast).
    while let Ok(ev) = rx.recv() {
        let last = matches!(ev, Event::ShuttingDown);
        send(&mut conn, &ServerFrame::Event(ev))?;
        if last {
            return conn.flush();
        }
    }
    Ok(())
}
