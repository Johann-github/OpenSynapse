//! Protocol 2.5 command table: `[default data_size, class, id]`.
//!
//! Source: `vault/Protocol/Commands 2.5.md`. Get = set | 0x80 (observed), with
//! default data sizes as listed there. Only commands OpenSynapse uses are named here.

/// One protocol 2.5 command: class, ID and default `data_size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Command {
    pub class: u8,
    pub id: u8,
    pub size: u8,
}

impl Command {
    pub const fn new(class: u8, id: u8, size: u8) -> Self {
        Self { class, id, size }
    }

    /// Bit 7 of the ID marks a read.
    pub const fn is_get(self) -> bool {
        self.id & 0x80 != 0
    }
}

macro_rules! commands {
    ($($(#[$m:meta])* $name:ident = ($class:expr, $id:expr, $size:expr);)*) => {
        $($(#[$m])* pub const $name: Command = Command::new($class, $id, $size);)*
    };
}

commands! {
    // Device info / system (class 0x00)
    GET_FIRMWARE_VERSION = (0x00, 0x81, 2);
    GET_SERIAL = (0x00, 0x82, 22);
    GET_PROTOCOL_VERSION = (0x00, 0x83, 2);
    GET_DEVICE_MODE = (0x00, 0x84, 2);
    SET_DEVICE_MODE = (0x00, 0x04, 2);
    GET_EDITION = (0x00, 0x86, 3);
    GET_EXTENDED_FIRMWARE_VERSION = (0x00, 0x87, 4);

    // Polling rate
    GET_POLLING_PERIOD = (0x00, 0x85, 1);
    SET_POLLING_PERIOD = (0x00, 0x05, 1);
    GET_HYPER_POLLING = (0x00, 0xC0, 2);
    SET_HYPER_POLLING = (0x00, 0x40, 2);

    // Wireless
    GET_WIRELESS_STATUS = (0x00, 0xB8, 1);

    // Buttons (class 0x02)
    GET_BUTTON_ASSIGNMENT = (0x02, 0x8C, 80);
    /// Synapse sends data_size 10 (capture, 2026-11-09); longer data grows it.
    SET_BUTTON_ASSIGNMENT = (0x02, 0x0C, 10);

    // Legacy LED (class 0x03)
    GET_LED_STATE = (0x03, 0x80, 3);
    SET_LED_STATE = (0x03, 0x00, 3);
    GET_LED_RGB = (0x03, 0x81, 5);
    SET_LED_RGB = (0x03, 0x01, 5);
    GET_LED_BRIGHTNESS = (0x03, 0x83, 3);
    SET_LED_BRIGHTNESS = (0x03, 0x03, 3);

    // DPI (class 0x04)
    GET_DPI = (0x04, 0x85, 7);
    SET_DPI = (0x04, 0x05, 7);
    GET_DPI_STAGES = (0x04, 0x86, 80);
    /// data_size is `7·n + 3`; the default here is a placeholder.
    SET_DPI_STAGES = (0x04, 0x06, 80);

    // Profiles (class 0x05)
    GET_PROFILE_COUNT = (0x05, 0x80, 1);
    GET_ACTIVE_PROFILE = (0x05, 0x84, 1);
    SET_ACTIVE_PROFILE = (0x05, 0x04, 1);
    GET_MAX_PROFILES = (0x05, 0x8A, 1);

    // Power (class 0x07)
    GET_BATTERY_LEVEL = (0x07, 0x80, 2);
    GET_LOW_BATTERY_THRESHOLD = (0x07, 0x81, 1);
    SET_LOW_BATTERY_THRESHOLD = (0x07, 0x01, 1);
    GET_SLEEP_TIME = (0x07, 0x83, 2);
    SET_SLEEP_TIME = (0x07, 0x03, 2);
    GET_CHARGING_STATUS = (0x07, 0x84, 2);
    GET_CONNECTION_QUALITY = (0x07, 0x96, 1);

    // Lighting, new style (class 0x0F)
    /// data_size is `6 + 3·colors`.
    SET_EFFECT = (0x0F, 0x02, 80);
    GET_EFFECT = (0x0F, 0x82, 80);
    SET_BRIGHTNESS = (0x0F, 0x04, 3);
    GET_BRIGHTNESS = (0x0F, 0x84, 3);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_set_pairs_differ_by_bit7() {
        let pairs = [
            (GET_DEVICE_MODE, SET_DEVICE_MODE),
            (GET_POLLING_PERIOD, SET_POLLING_PERIOD),
            (GET_HYPER_POLLING, SET_HYPER_POLLING),
            (GET_BUTTON_ASSIGNMENT, SET_BUTTON_ASSIGNMENT),
            (GET_LED_STATE, SET_LED_STATE),
            (GET_LED_RGB, SET_LED_RGB),
            (GET_LED_BRIGHTNESS, SET_LED_BRIGHTNESS),
            (GET_DPI, SET_DPI),
            (GET_DPI_STAGES, SET_DPI_STAGES),
            (GET_ACTIVE_PROFILE, SET_ACTIVE_PROFILE),
            (GET_LOW_BATTERY_THRESHOLD, SET_LOW_BATTERY_THRESHOLD),
            (GET_SLEEP_TIME, SET_SLEEP_TIME),
            (GET_EFFECT, SET_EFFECT),
            (GET_BRIGHTNESS, SET_BRIGHTNESS),
        ];
        for (get, set) in pairs {
            assert_eq!(get.class, set.class);
            assert_eq!(get.id, set.id | 0x80, "{get:?}");
            assert!(get.is_get() && !set.is_get());
        }
    }
}
