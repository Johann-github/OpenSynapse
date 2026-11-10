# opensynapse-protocol

Pure encode/decode of Razer protocol 2.5 reports. No I/O, fully unit-tested.

- `report` — 90-byte frame, XOR CRC, status codes + retry disposition, rolling transaction ID.
  Implements [[Protocol/Report Format 2.5]].
- `command` — command table `(class, id, default data_size)`. Implements [[Protocol/Commands 2.5]].
- `device`, `dpi`, `polling`, `power`, `profile`, `buttons`, `lighting` — typed request
  builders (`get_*`/`set_*` → `Request`) and reply parsers (`parse_*(report.args())`).
- `hid_usage` — key names ↔ USB-HID keyboard usage IDs and modifier bits (public HID spec).
- `event` — hardware-event decoder (records 5/9). Implements [[Protocol/Hardware Events]].

Single source of truth: other crates never build report bytes themselves.

Vault: [[Architecture/Crates]]
