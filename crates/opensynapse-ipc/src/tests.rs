use std::io::{self, Cursor};

use opensynapse_protocol::buttons::{ButtonAssignment, Mode};
use opensynapse_protocol::device::{DeviceMode, FirmwareVersion};
use opensynapse_protocol::dpi::{Dpi, DpiStage, DpiStages, Storage};
use opensynapse_protocol::power::{BatteryLevel, ChargingStatus};

use crate::codec::*;
use crate::frame::*;
use crate::msg::*;

// ---- sample values ----

fn button(mode: Mode) -> ButtonAssignment {
    ButtonAssignment {
        profile: 1,
        button: 5,
        mode,
        function: 2,
        data: vec![0x02, 0x04],
    }
}

fn stages() -> DpiStages {
    DpiStages {
        active: 2,
        stages: vec![
            DpiStage {
                id: 1,
                dpi: Dpi::uniform(800),
            },
            DpiStage {
                id: 2,
                dpi: Dpi { x: 1600, y: 1200 },
            },
        ],
    }
}

fn settings() -> Vec<WireSetting> {
    vec![
        WireSetting::Dpi(Dpi { x: 1600, y: 800 }),
        WireSetting::Polling {
            hz: 8000,
            variant: PollingVariant::Hyper,
        },
        WireSetting::Polling {
            hz: 1000,
            variant: PollingVariant::Period,
        },
        WireSetting::Sleep(300),
        WireSetting::Button(button(Mode::Normal)),
        WireSetting::Button(button(Mode::HyperShift)),
        WireSetting::DpiStages {
            storage: Storage::Volatile,
            stages: stages(),
        },
        WireSetting::DpiStages {
            storage: Storage::Persistent,
            stages: DpiStages {
                active: 0,
                stages: vec![],
            },
        },
    ]
}

fn device_refs() -> Vec<DeviceRef> {
    vec![
        DeviceRef::Pid(0x00B8),
        DeviceRef::Path(r"\\?\hid#vid_1532&pid_00b8".into()),
    ]
}

fn device_info(name: Option<&str>) -> DeviceInfo {
    DeviceInfo {
        path: r"\\?\hid#vid_1532&pid_00b8&mi_02".into(),
        pid: 0x00B8,
        name: name.map(str::to_owned),
        protocol: "2.5".into(),
        supported: name.is_some(),
    }
}

fn opts() -> WireWriteOpts {
    WireWriteOpts {
        confirmed: true,
        allow_primary: false,
        restore: true,
    }
}

fn requests() -> Vec<Request> {
    let mut v = vec![Request::Ping, Request::ListDevices, Request::Shutdown];
    for device in device_refs() {
        v.push(Request::ReadSnapshot {
            device: device.clone(),
        });
        for setting in settings() {
            v.push(Request::Write {
                device: device.clone(),
                setting,
                opts: opts(),
            });
        }
        v.push(Request::ApplyProfile {
            device,
            toml: "[dpi]\nactive = 800 # ünïcode\n".into(),
            dry_run: true,
            all: false,
            opts: WireWriteOpts::default(),
        });
    }
    v
}

fn snapshot_ok() -> WireSnapshot {
    WireSnapshot {
        firmware: Ok(FirmwareVersion { major: 1, minor: 3 }),
        serial: Ok("PM2345H0000001".into()),
        mode: Ok((DeviceMode::Normal, 0)),
        battery: Ok(BatteryLevel(200)),
        charging: Ok(ChargingStatus::Charging),
        dpi: Ok(Dpi::uniform(800)),
        stages: Ok(stages()),
        polling: Ok((1000, PollingVariant::Period)),
        sleep: Ok(300),
        buttons_normal: Ok(vec![button(Mode::Normal), button(Mode::Normal)]),
        buttons_hypershift: Ok(vec![]),
    }
}

fn e<T>() -> Field<T> {
    Err("timeout".to_string())
}

fn snapshot_err() -> WireSnapshot {
    WireSnapshot {
        firmware: e(),
        serial: e(),
        mode: e(),
        battery: e(),
        charging: e(),
        dpi: e(),
        stages: e(),
        polling: e(),
        sleep: e(),
        buttons_normal: e(),
        buttons_hypershift: e(),
    }
}

const DEVICE_MODES: [DeviceMode; 11] = [
    DeviceMode::Normal,
    DeviceMode::Bootloader,
    DeviceMode::Testing,
    DeviceMode::Driver,
    DeviceMode::Download,
    DeviceMode::Pairing,
    DeviceMode::Standby,
    DeviceMode::RfTest,
    DeviceMode::Retail,
    DeviceMode::Other(0x42),
    DeviceMode::Other(0),
];

const CHARGING: [ChargingStatus; 9] = [
    ChargingStatus::NotCharging,
    ChargingStatus::Charging,
    ChargingStatus::Failed,
    ChargingStatus::ForeignObject,
    ChargingStatus::FastCharging,
    ChargingStatus::Full,
    ChargingStatus::LimitReached,
    ChargingStatus::Other(0x99),
    ChargingStatus::Other(0),
];

const ERROR_KINDS: [ErrorKind; 7] = [
    ErrorKind::BadRequest,
    ErrorKind::Version,
    ErrorKind::NoSuchDevice,
    ErrorKind::Ambiguous,
    ErrorKind::Unsupported,
    ErrorKind::Conflict,
    ErrorKind::Device,
];

fn replies() -> Vec<Reply> {
    let mut v = vec![
        Reply::Pong {
            agent_version: "0.1.0".into(),
        },
        Reply::Devices {
            list: vec![
                device_info(Some("Razer Viper V3 HyperSpeed")),
                device_info(None),
            ],
            conflict: Some("RazerAppEngine.exe".into()),
        },
        Reply::Devices {
            list: vec![],
            conflict: None,
        },
        Reply::Snapshot(snapshot_ok()),
        Reply::Snapshot(snapshot_err()),
        Reply::ProfileApplied {
            lines: vec!["dpi 800".into(), String::new()],
        },
        Reply::ShuttingDown,
        Reply::Written(Err(WriteFailed {
            old: None,
            message: "no reply".into(),
        })),
    ];
    for m in DEVICE_MODES {
        let mut s = snapshot_ok();
        s.mode = Ok((m, 7));
        v.push(Reply::Snapshot(s));
    }
    for c in CHARGING {
        let mut s = snapshot_ok();
        s.charging = Ok(c);
        v.push(Reply::Snapshot(s));
    }
    for s in settings() {
        v.push(Reply::Written(Ok(WriteDone {
            old: s.clone(),
            new: s.clone(),
            now: s.clone(),
            verified: true,
        })));
        v.push(Reply::Written(Err(WriteFailed {
            old: Some(s),
            message: "busy".into(),
        })));
    }
    v
}

fn events() -> Vec<Event> {
    vec![
        Event::DeviceArrived(device_info(Some("Viper"))),
        Event::DeviceRemoved {
            path: "p".into(),
            pid: 0x00B8,
        },
        Event::Battery {
            path: "p".into(),
            level: 255,
            charging: 1,
        },
        Event::DpiChanged {
            path: "p".into(),
            x: 400,
            y: 26000,
        },
        Event::Hardware {
            path: "p".into(),
            id: 0x02,
            payload: vec![0, 1, 2, 255],
        },
        Event::Error {
            message: "hotplug failed".into(),
        },
        Event::ShuttingDown,
    ]
}

fn client_frames() -> Vec<ClientFrame> {
    let mut v = vec![
        ClientFrame::Hello {
            version: PROTOCOL_VERSION,
            role: Role::Control,
        },
        ClientFrame::Hello {
            version: 0xBEEF,
            role: Role::Subscribe,
        },
    ];
    for (i, req) in requests().into_iter().enumerate() {
        v.push(ClientFrame::Request {
            id: u32::MAX - i as u32,
            req,
        });
    }
    v
}

fn server_frames() -> Vec<ServerFrame> {
    let mut v = vec![ServerFrame::Welcome {
        version: PROTOCOL_VERSION,
        agent_pid: 4242,
    }];
    for (i, reply) in replies().into_iter().enumerate() {
        v.push(ServerFrame::Response {
            id: i as u32,
            result: Ok(reply),
        });
    }
    for kind in ERROR_KINDS {
        v.push(ServerFrame::Response {
            id: 7,
            result: Err(IpcError {
                kind,
                message: "nope".into(),
            }),
        });
    }
    v.extend(events().into_iter().map(ServerFrame::Event));
    v
}

// ---- codec ----

#[test]
fn client_frames_round_trip() {
    for f in client_frames() {
        let b = encode_client(&f);
        assert_eq!(decode_client(&b), Ok(f));
    }
}

#[test]
fn server_frames_round_trip() {
    for f in server_frames() {
        let b = encode_server(&f);
        assert_eq!(decode_server(&b), Ok(f));
    }
}

#[test]
fn every_truncation_is_an_error() {
    for f in client_frames() {
        let b = encode_client(&f);
        for n in 0..b.len() {
            assert!(decode_client(&b[..n]).is_err(), "{f:?} cut at {n}");
        }
    }
    for f in server_frames() {
        let b = encode_server(&f);
        for n in 0..b.len() {
            assert!(decode_server(&b[..n]).is_err(), "{f:?} cut at {n}");
        }
    }
}

#[test]
fn known_layouts() {
    assert_eq!(
        encode_client(&ClientFrame::Hello {
            version: 1,
            role: Role::Subscribe
        }),
        [0, 1, 0, 1]
    );
    assert_eq!(
        encode_client(&ClientFrame::Request {
            id: 0x0102_0304,
            req: Request::ReadSnapshot {
                device: DeviceRef::Path("ab".into())
            }
        }),
        [1, 4, 3, 2, 1, 2, 1, 2, 0, 0, 0, b'a', b'b']
    );
    assert_eq!(
        encode_server(&ServerFrame::Response {
            id: 9,
            result: Err(IpcError {
                kind: ErrorKind::Device,
                message: String::new()
            })
        }),
        [1, 9, 0, 0, 0, 1, 6, 0, 0, 0, 0]
    );
}

#[test]
fn empty_body_is_truncated() {
    assert_eq!(decode_client(&[]), Err(DecodeError::Truncated));
    assert_eq!(decode_server(&[]), Err(DecodeError::Truncated));
}

#[test]
fn unknown_tags() {
    assert_eq!(
        decode_client(&[9]),
        Err(DecodeError::UnknownTag {
            what: "ClientFrame",
            tag: 9
        })
    );
    assert_eq!(
        decode_server(&[3]),
        Err(DecodeError::UnknownTag {
            what: "ServerFrame",
            tag: 3
        })
    );
    // Hello with role 2.
    assert_eq!(
        decode_client(&[0, 1, 0, 2]),
        Err(DecodeError::UnknownTag {
            what: "Role",
            tag: 2
        })
    );
    // Request id 0, request tag 6.
    assert_eq!(
        decode_client(&[1, 0, 0, 0, 0, 6]),
        Err(DecodeError::UnknownTag {
            what: "Request",
            tag: 6
        })
    );
    // Response id 0, Result tag 2.
    assert_eq!(
        decode_server(&[1, 0, 0, 0, 0, 2]),
        Err(DecodeError::UnknownTag {
            what: "Result",
            tag: 2
        })
    );
    // Response Err with ErrorKind 7.
    assert_eq!(
        decode_server(&[1, 0, 0, 0, 0, 1, 7]),
        Err(DecodeError::UnknownTag {
            what: "ErrorKind",
            tag: 7
        })
    );
    // Event tag 7.
    assert_eq!(
        decode_server(&[2, 7]),
        Err(DecodeError::UnknownTag {
            what: "Event",
            tag: 7
        })
    );
}

#[test]
fn invalid_bool() {
    let mut b = encode_server(&ServerFrame::Event(Event::DeviceArrived(device_info(None))));
    let last = b.len() - 1;
    b[last] = 2;
    assert_eq!(decode_server(&b), Err(DecodeError::InvalidBool(2)));
}

#[test]
fn invalid_utf8() {
    // Event::Error with a 2-byte string that is not UTF-8.
    let b = [2, 5, 2, 0, 0, 0, 0xC3, 0x28];
    assert_eq!(decode_server(&b), Err(DecodeError::InvalidUtf8));
}

#[test]
fn length_beyond_remaining() {
    // Event::Error announcing a 0xFFFF_FFFF byte string.
    let b = [2, 5, 0xFF, 0xFF, 0xFF, 0xFF, b'x'];
    assert_eq!(decode_server(&b), Err(DecodeError::Length(u32::MAX)));
    // Pong with length 2 but only 1 byte left.
    let b = [1, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, b'x'];
    assert_eq!(decode_server(&b), Err(DecodeError::Length(2)));
    // ProfileApplied announcing a huge list count.
    let b = [1, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0x10];
    assert_eq!(decode_server(&b), Err(DecodeError::Length(0x1000_0000)));
}

#[test]
fn trailing_bytes() {
    let mut b = encode_client(&ClientFrame::Request {
        id: 1,
        req: Request::Ping,
    });
    b.extend_from_slice(&[0, 0]);
    assert_eq!(decode_client(&b), Err(DecodeError::Trailing(2)));
    let mut b = encode_server(&ServerFrame::Event(Event::ShuttingDown));
    b.push(0);
    assert_eq!(decode_server(&b), Err(DecodeError::Trailing(1)));
}

// ---- framing ----

#[test]
fn frames_round_trip_over_cursor() {
    let mut buf = Vec::new();
    write_frame(&mut buf, b"hello").unwrap();
    write_frame(&mut buf, b"").unwrap();
    assert_eq!(&buf[..4], &5u32.to_le_bytes());
    let mut r = Cursor::new(buf);
    assert_eq!(read_frame(&mut r).unwrap(), b"hello");
    assert_eq!(read_frame(&mut r).unwrap(), b"");
    let e = read_frame(&mut r).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn frame_max_size() {
    let body = vec![7u8; MAX_FRAME];
    let mut buf = Vec::new();
    write_frame(&mut buf, &body).unwrap();
    assert_eq!(read_frame(&mut Cursor::new(buf)).unwrap(), body);

    let e = write_frame(&mut Vec::new(), &vec![0u8; MAX_FRAME + 1]).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::InvalidInput);

    let header = ((MAX_FRAME + 1) as u32).to_le_bytes();
    let e = read_frame(&mut Cursor::new(header)).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn frame_cut_short_is_eof() {
    // Partial header.
    let e = read_frame(&mut Cursor::new([5u8, 0])).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
    // Header announces 5, body has 2.
    let e = read_frame(&mut Cursor::new([5u8, 0, 0, 0, 1, 2])).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
}

// ---- Windows named pipe ----

#[cfg(windows)]
mod pipe {
    use std::io;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use crate::client::Client;
    use crate::codec::{decode_client, encode_server};
    use crate::frame::{read_frame, write_frame};
    use crate::msg::*;
    use crate::transport::{self, Conn, Listener};

    fn unique_name() -> String {
        static N: AtomicU32 = AtomicU32::new(0);
        format!(
            r"\\.\pipe\opensynapse-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn recv(conn: &mut Conn) -> io::Result<ClientFrame> {
        let body = read_frame(conn)?;
        Ok(decode_client(&body).expect("valid client frame"))
    }

    fn send(conn: &mut Conn, f: &ServerFrame) {
        write_frame(conn, &encode_server(f)).unwrap();
    }

    /// Accepts one connection, welcomes it with `version`, then answers every
    /// request with `Pong { agent_version: "<id>" }` until the client hangs up.
    fn echo_server(name: &str, version: u16) -> thread::JoinHandle<Role> {
        let mut l = Listener::bind(name).unwrap();
        thread::spawn(move || {
            let mut c = l.accept().unwrap();
            let role = match recv(&mut c).unwrap() {
                ClientFrame::Hello { role, .. } => role,
                other => panic!("expected Hello, got {other:?}"),
            };
            send(
                &mut c,
                &ServerFrame::Welcome {
                    version,
                    agent_pid: std::process::id(),
                },
            );
            if role == Role::Subscribe {
                for e in [
                    Event::Error {
                        message: "one".into(),
                    },
                    Event::ShuttingDown,
                ] {
                    send(&mut c, &ServerFrame::Event(e));
                }
                return role;
            }
            while let Ok(f) = recv(&mut c) {
                let ClientFrame::Request { id, .. } = f else {
                    panic!("expected Request");
                };
                send(
                    &mut c,
                    &ServerFrame::Response {
                        id,
                        result: Ok(Reply::Pong {
                            agent_version: id.to_string(),
                        }),
                    },
                );
            }
            role
        })
    }

    #[test]
    fn default_name_contains_user_sid() {
        let name = transport::default_pipe_name().unwrap();
        assert!(
            name.starts_with(r"\\.\pipe\opensynapse-agent-S-1-"),
            "{name}"
        );
    }

    #[test]
    fn raw_handshake_round_trip() {
        let name = unique_name();
        let mut l = Listener::bind(&name).unwrap();
        let server = thread::spawn(move || {
            let mut c = l.accept().unwrap();
            let hello = recv(&mut c).unwrap();
            send(
                &mut c,
                &ServerFrame::Welcome {
                    version: PROTOCOL_VERSION,
                    agent_pid: 1,
                },
            );
            hello
        });
        let mut c = transport::connect(&name).unwrap();
        write_frame(
            &mut c,
            &crate::codec::encode_client(&ClientFrame::Hello {
                version: PROTOCOL_VERSION,
                role: Role::Control,
            }),
        )
        .unwrap();
        let welcome = crate::codec::decode_server(&read_frame(&mut c).unwrap()).unwrap();
        assert_eq!(
            welcome,
            ServerFrame::Welcome {
                version: PROTOCOL_VERSION,
                agent_pid: 1
            }
        );
        assert_eq!(
            server.join().unwrap(),
            ClientFrame::Hello {
                version: PROTOCOL_VERSION,
                role: Role::Control
            }
        );
        // Server hung up: clean EOF.
        let e = read_frame(&mut c).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn second_bind_is_addr_in_use() {
        let name = unique_name();
        let _first = Listener::bind(&name).unwrap();
        let e = Listener::bind(&name).err().expect("second bind must fail");
        assert_eq!(e.kind(), io::ErrorKind::AddrInUse);
    }

    #[test]
    fn missing_pipe_is_not_found() {
        let e = transport::connect(&unique_name()).err().expect("no server");
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        let e = Client::connect_to(&unique_name()).err().expect("no server");
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn waker_unblocks_accept() {
        let name = unique_name();
        let mut l = Listener::bind(&name).unwrap();
        let waker = l.waker();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let r = l.accept().map(|_| ());
            tx.send(r.is_ok()).unwrap();
        });
        thread::sleep(Duration::from_millis(50));
        waker.wake();
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
    }

    #[test]
    fn several_connections_in_a_row() {
        let name = unique_name();
        let mut l = Listener::bind(&name).unwrap();
        let server = thread::spawn(move || {
            let mut conns = Vec::new();
            for _ in 0..3 {
                conns.push(l.accept().unwrap());
            }
            conns.len()
        });
        let clients: Vec<_> = (0..3).map(|_| transport::connect(&name).unwrap()).collect();
        assert_eq!(server.join().unwrap(), 3);
        drop(clients);
    }

    #[test]
    fn client_call_matches_ids() {
        let name = unique_name();
        let server = echo_server(&name, PROTOCOL_VERSION);
        let mut c = Client::connect_to(&name).unwrap();
        assert_eq!(c.agent_pid(), std::process::id());
        for want in ["1", "2", "3"] {
            assert_eq!(
                c.call(Request::Ping).unwrap(),
                Ok(Reply::Pong {
                    agent_version: want.into()
                })
            );
        }
        drop(c);
        assert_eq!(server.join().unwrap(), Role::Control);
    }

    #[test]
    fn client_rejects_other_version() {
        let name = unique_name();
        let server = echo_server(&name, PROTOCOL_VERSION + 1);
        let e = Client::connect_to(&name).err().expect("version mismatch");
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        server.join().unwrap();
    }

    #[test]
    fn subscription_yields_events_until_close() {
        let name = unique_name();
        let server = echo_server(&name, PROTOCOL_VERSION);
        let events: Vec<Event> = Client::subscribe_to(&name)
            .unwrap()
            .collect::<io::Result<_>>()
            .unwrap();
        assert_eq!(
            events,
            [
                Event::Error {
                    message: "one".into()
                },
                Event::ShuttingDown
            ]
        );
        assert_eq!(server.join().unwrap(), Role::Subscribe);
    }
}
