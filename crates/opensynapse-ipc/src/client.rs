//! Blocking client: control calls and event subscriptions.
//!
//! Each connection starts with `Hello { version, role }` → `Welcome { version,
//! agent_pid }`. A version mismatch is an `InvalidData` error; an agent that
//! refuses the hello with an error response gives an `Other` error carrying its
//! message.

use std::io;

use crate::codec::{decode_server, encode_client};
use crate::frame::{read_frame, write_frame};
use crate::msg::{
    ClientFrame, Event, IpcError, PROTOCOL_VERSION, Reply, Request, Role, ServerFrame,
};
use crate::transport::{self, Conn};

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn send(conn: &mut Conn, f: &ClientFrame) -> io::Result<()> {
    write_frame(conn, &encode_client(f))
}

fn recv(conn: &mut Conn) -> io::Result<ServerFrame> {
    let body = read_frame(conn)?;
    decode_server(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Connect, say hello with `role`, check the welcome. Returns the agent's pid.
fn handshake(name: &str, role: Role) -> io::Result<(Conn, u32)> {
    let mut conn = transport::connect(name)?;
    send(
        &mut conn,
        &ClientFrame::Hello {
            version: PROTOCOL_VERSION,
            role,
        },
    )?;
    match recv(&mut conn)? {
        ServerFrame::Welcome { version, agent_pid } if version == PROTOCOL_VERSION => {
            Ok((conn, agent_pid))
        }
        ServerFrame::Welcome { version, .. } => Err(invalid(format!(
            "agent speaks IPC protocol {version}, this client {PROTOCOL_VERSION}"
        ))),
        ServerFrame::Response { result: Err(e), .. } => Err(io::Error::other(format!(
            "agent refused the connection: {}",
            e.message
        ))),
        other => Err(invalid(format!("expected Welcome, got {other:?}"))),
    }
}

/// A `Control` connection to the agent.
pub struct Client {
    conn: Conn,
    next_id: u32,
    agent_pid: u32,
}

impl Client {
    /// Connect to the current user's agent.
    pub fn connect() -> io::Result<Self> {
        Self::connect_to(&transport::default_pipe_name()?)
    }

    pub fn connect_to(name: &str) -> io::Result<Self> {
        let (conn, agent_pid) = handshake(name, Role::Control)?;
        Ok(Self {
            conn,
            next_id: 1,
            agent_pid,
        })
    }

    /// Process id the agent reported in its welcome.
    pub fn agent_pid(&self) -> u32 {
        self.agent_pid
    }

    /// Send one request and wait for its response.
    pub fn call(&mut self, req: Request) -> io::Result<Result<Reply, IpcError>> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        send(&mut self.conn, &ClientFrame::Request { id, req })?;
        match recv(&mut self.conn)? {
            ServerFrame::Response { id: got, result } if got == id => Ok(result),
            ServerFrame::Response { id: got, .. } => Err(invalid(format!(
                "response id {got} does not match request id {id}"
            ))),
            other => Err(invalid(format!("expected Response, got {other:?}"))),
        }
    }

    /// Open a `Subscribe` connection to the current user's agent.
    pub fn subscribe() -> io::Result<Subscription> {
        Self::subscribe_to(&transport::default_pipe_name()?)
    }

    /// Open a `Subscribe` connection.
    pub fn subscribe_to(name: &str) -> io::Result<Subscription> {
        let (conn, _) = handshake(name, Role::Subscribe)?;
        Ok(Subscription { conn: Some(conn) })
    }
}

/// Events pushed by the agent, until it closes the connection. A clean close
/// ends the iterator; after an error it yields nothing more.
pub struct Subscription {
    conn: Option<Conn>,
}

impl Iterator for Subscription {
    type Item = io::Result<Event>;

    fn next(&mut self) -> Option<Self::Item> {
        let conn = self.conn.as_mut()?;
        let item = match recv(conn) {
            Ok(ServerFrame::Event(e)) => return Some(Ok(e)),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => None,
            Err(e) => Some(Err(e)),
            Ok(other) => Some(Err(invalid(format!("expected Event, got {other:?}")))),
        };
        self.conn = None;
        item
    }
}
