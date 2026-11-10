# opensynapse-protocol

Pure encode/decode of Razer protocol 2.5 reports. No I/O, fully unit-tested.

- `report` — 90-byte frame, XOR CRC, status codes + retry disposition, rolling transaction ID.
  Implements [[Protocol/Report Format 2.5]].
- `command` — command table `(class, id, default data_size)`. Implements [[Protocol/Commands 2.5]].
- `device`, `dpi`, `polling`, `power`, `profile`, `buttons`, `lighting` — typed request
  builders (`get_*`/`set_*` → `Request`) and reply parsers (`parse_*(report.args())`).
- `buttons`: byte 2 of 02/0C and 02/8C is the **mode** (`Mode::Normal` 0,
  `Mode::HyperShift` 1; Synapse 4 code). `parse_button_assignment(args, requested)`
  returns the requested mode plus the raw echoed byte (`ButtonReply::mode_echo`),
  because the Viper V3 HyperSpeed ignores the requested mode in its 02/8C reply
  (hardware). 02/8E `get_button_assignment_list` / `parse_button_assignment_list` give
  the raw reply only (layout unknown).
- `hid_usage` — key names ↔ USB-HID keyboard usage IDs and modifier bits (public HID spec).
- `event` — hardware-event decoder (records 5/9). Implements [[Protocol/Hardware Events]].

Single source of truth: other crates never build report bytes themselves.

Vault: [[Architecture/Crates]]
