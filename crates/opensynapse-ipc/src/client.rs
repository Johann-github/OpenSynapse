//! Blocking client: control calls and event subscriptions.

use std::io;

use crate::msg::{Event, IpcError, Reply, Request};
use crate::transport::Conn;

/// A `Control` connection to the agent.
pub struct Client {
    _conn: Conn,
}

impl Client {
    /// Connect to the current user's agent.
    pub fn connect() -> io::Result<Self> {
        todo!("ipc client: Client::connect")
    }

    pub fn connect_to(name: &str) -> io::Result<Self> {
        let _ = name;
        todo!("ipc client: Client::connect_to")
    }

    /// Send one request and wait for its response.
    pub fn call(&mut self, req: Request) -> io::Result<Result<Reply, IpcError>> {
        let _ = req;
        todo!("ipc client: Client::call")
    }

    /// Open a `Subscribe` connection.
    pub fn subscribe_to(name: &str) -> io::Result<Subscription> {
        let _ = name;
        todo!("ipc client: Client::subscribe_to")
    }
}

/// Events pushed by the agent, until it closes the connection.
pub struct Subscription {
    _conn: Conn,
}

impl Iterator for Subscription {
    type Item = io::Result<Event>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!("ipc client: Subscription::next")
    }
}
