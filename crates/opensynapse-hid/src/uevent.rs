//! Pure decoding of Linux uevent datagrams (netlink `NETLINK_KOBJECT_UEVENT`) into
//! [`HotplugEvent`]s. No I/O, so it compiles and is tested on every platform; only
//! `linux_hotplug.rs` uses it at run time.
//!
//! Decision and message formats: `vault/Decisions/ADR-0010 Linux hotplug.md`.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::hotplug::HotplugEvent;

/// Prefix of a message from the udev daemon (multicast group 2). Kernel messages
/// (group 1) start with `ACTION@DEVPATH` instead.
const UDEV_PREFIX: &[u8] = b"libudev\0";

/// Decode one datagram. Returns an event only for `hidraw` nodes of vendor `vid`
/// with action `add` or `remove`; everything else is `None`.
pub(crate) fn parse(msg: &[u8], vid: u16) -> Option<HotplugEvent> {
    let props = properties(msg)?;
    let (mut action, mut subsystem, mut devpath, mut devname) = (None, None, None, None);
    for field in props.split(|&b| b == 0) {
        let Ok(field) = std::str::from_utf8(field) else {
            continue;
        };
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        match key {
            "ACTION" => action = Some(value),
            "SUBSYSTEM" => subsystem = Some(value),
            "DEVPATH" => devpath = Some(value),
            "DEVNAME" => devname = Some(value),
            _ => {}
        }
    }
    if subsystem? != "hidraw" || devpath_vendor(devpath?)? != vid {
        return None;
    }
    let path = node_path(devname?);
    match action? {
        "add" => Some(HotplugEvent::Arrived { path }),
        "remove" => Some(HotplugEvent::Removed { path }),
        _ => None,
    }
}

/// The `KEY=VALUE\0…` block of a datagram. udev messages carry a binary header
/// whose `properties_off` / `properties_len` (native-endian u32 at bytes 16 and 20)
/// locate the block; kernel messages are the block themselves.
fn properties(msg: &[u8]) -> Option<&[u8]> {
    if !msg.starts_with(UDEV_PREFIX) {
        return Some(msg);
    }
    let word = |at: usize| -> Option<usize> {
        let bytes: [u8; 4] = msg.get(at..at + 4)?.try_into().ok()?;
        usize::try_from(u32::from_ne_bytes(bytes)).ok()
    };
    let (off, len) = (word(16)?, word(20)?);
    msg.get(off..off.checked_add(len)?)
}

/// `/dev/hidrawN` from `DEVNAME`: the kernel sends `hidrawN`, udev the full path.
fn node_path(devname: &str) -> String {
    if devname.starts_with('/') {
        devname.to_owned()
    } else {
        format!("/dev/{devname}")
    }
}

/// Vendor ID from the HID device directory in a sysfs `DEVPATH`, named
/// `BBBB:VVVV:PPPP.NNNN` (bus, vendor, product, instance; upper-case hex), e.g.
/// `/devices/…/0003:1532:00B8.0001/hidraw/hidraw0`. The last such component wins.
fn devpath_vendor(devpath: &str) -> Option<u16> {
    devpath.split('/').rev().find_map(|c| {
        let b = c.as_bytes();
        let shape = b.len() == 19
            && b[4] == b':'
            && b[9] == b':'
            && b[14] == b'.'
            && [0..4, 5..9, 10..14, 15..19]
                .into_iter()
                .all(|r| b[r].iter().all(u8::is_ascii_hexdigit));
        if shape {
            u16::from_str_radix(&c[5..9], 16).ok()
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VID: u16 = 0x1532;
    const DEVPATH: &str =
        "/devices/pci0000:00/0000:00:14.0/usb1/1-2/1-2:1.0/0003:1532:00B8.0001/hidraw/hidraw0";

    fn kernel(action: &str, subsystem: &str, devpath: &str, devname: &str) -> Vec<u8> {
        format!(
            "{action}@{devpath}\0ACTION={action}\0DEVPATH={devpath}\0SUBSYSTEM={subsystem}\0\
             MAJOR=241\0MINOR=0\0DEVNAME={devname}\0SEQNUM=4711\0"
        )
        .into_bytes()
    }

    /// A udev-format datagram: 40-byte header (layout as in the ADR), then the block.
    fn udev(props: &str) -> Vec<u8> {
        let header_len = 40u32;
        let mut m = Vec::new();
        m.extend_from_slice(UDEV_PREFIX);
        m.extend_from_slice(&0xfeed_cafe_u32.to_be_bytes());
        m.extend_from_slice(&header_len.to_ne_bytes());
        m.extend_from_slice(&header_len.to_ne_bytes()); // properties_off
        m.extend_from_slice(&(props.len() as u32).to_ne_bytes()); // properties_len
        // Hash / bloom filter words; non-zero last byte must not leak into the block.
        m.extend_from_slice(&[0xab; 16]);
        assert_eq!(m.len(), header_len as usize);
        m.extend_from_slice(props.as_bytes());
        m
    }

    #[test]
    fn kernel_add_and_remove() {
        let add = kernel("add", "hidraw", DEVPATH, "hidraw0");
        assert_eq!(
            parse(&add, VID),
            Some(HotplugEvent::Arrived {
                path: "/dev/hidraw0".into()
            })
        );
        let rm = kernel("remove", "hidraw", DEVPATH, "hidraw0");
        assert_eq!(
            parse(&rm, VID),
            Some(HotplugEvent::Removed {
                path: "/dev/hidraw0".into()
            })
        );
    }

    #[test]
    fn udev_message_uses_property_block() {
        let props = format!(
            "ACTION=add\0DEVPATH={DEVPATH}\0SUBSYSTEM=hidraw\0DEVNAME=/dev/hidraw3\0\
             SEQNUM=12\0USEC_INITIALIZED=123\0TAGS=:uaccess:seat:\0"
        );
        assert_eq!(
            parse(&udev(&props), VID),
            Some(HotplugEvent::Arrived {
                path: "/dev/hidraw3".into()
            })
        );
    }

    #[test]
    fn udev_header_out_of_range_is_ignored() {
        let mut m = udev("ACTION=add\0");
        m[20..24].copy_from_slice(&1000u32.to_ne_bytes());
        assert_eq!(parse(&m, VID), None);
        assert_eq!(parse(&m[..10], VID), None);
    }

    #[test]
    fn filters_vendor_subsystem_and_action() {
        let other = DEVPATH.replace(":1532:", ":046D:");
        assert_eq!(
            parse(&kernel("add", "hidraw", &other, "hidraw0"), VID),
            None
        );
        // The parent `hid` device event has no hidraw node.
        assert_eq!(parse(&kernel("add", "hid", DEVPATH, "hidraw0"), VID), None);
        assert_eq!(
            parse(&kernel("change", "hidraw", DEVPATH, "hidraw0"), VID),
            None
        );
        assert_eq!(
            parse(&kernel("bind", "hidraw", DEVPATH, "hidraw0"), VID),
            None
        );
        assert_eq!(parse(b"", VID), None);
        assert_eq!(parse(b"ACTION=add\0SUBSYSTEM=hidraw\0", VID), None);
        // Missing DEVNAME: no node to report.
        let no_name = format!("ACTION=add\0SUBSYSTEM=hidraw\0DEVPATH={DEVPATH}\0");
        assert_eq!(parse(no_name.as_bytes(), VID), None);
    }

    #[test]
    fn devpath_vendor_shapes() {
        assert_eq!(devpath_vendor(DEVPATH), Some(0x1532));
        // Bluetooth bus, lower-case hex.
        assert_eq!(
            devpath_vendor("/devices/virtual/misc/uhid/0005:1532:00ba.0007/hidraw/hidraw5"),
            Some(0x1532)
        );
        assert_eq!(devpath_vendor("/devices/virtual/misc/uhid"), None);
        assert_eq!(devpath_vendor("0003:1532:00B8.000"), None);
        assert_eq!(devpath_vendor("0003:15G2:00B8.0001"), None);
        assert_eq!(devpath_vendor(""), None);
    }
}
