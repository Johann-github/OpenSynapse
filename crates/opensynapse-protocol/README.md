# opensynapse-protocol

Pure encode/decode of Razer protocol 2.5 reports. No I/O, fully unit-tested.

- `report` — 90-byte frame, XOR CRC, status codes + retry disposition, rolling transaction ID.
  Implements [[Protocol/Report Format 2.5]].
- `command` — command table `(class, id, default data_size)`. Implements [[Protocol/Commands 2.5]].
- `device`, `dpi`, `polling`, `power`, `profile`, `buttons`, `lighting` — typed request
  builders (`get_*`/`set_*` → `Request`) and reply parsers (`parse_*(report.args())`).
- `buttons`: byte 2 of 02/0C and 02/8C is the **mode** (`Mode::Normal` 0,
  `Mode::HyperShift` 1; Synapse 4 code). The Viper V3 HyperSpeed keeps a separate
  slot per mode and answers 02/8C from the requested one (hardware, observed).
  `parse_button_assignment(args, requested)` returns the assignment with the requested
  mode plus the raw reply byte 2 (`ButtonReply::reply_byte2`); that byte is not the
  mode and its meaning is unknown. 02/8E `get_button_assignment_list` /
  `parse_button_assignment_list` return the raw reply; `ButtonAssignmentList::records()`
  splits it into `(button_id, function, data)` using 8-byte records
  `[buttonId, function, dataLen, data×5]`, ending at button ID 0 (layout inferred).
- `lighting` — `Mode` (off, static, breathing 1/2 colours, spectrum, wave) encoded for the
  extended family (`set_mode`, 0F/02; `Effect::mode()` decodes 0F/82) and the standard
  family (`standard::set_mode`, 03/0A); brightness in both families with Synapse's
  percent rounding; `Rgb` parses/prints `00ff00`; `region` names zone IDs. Implements
  [[Protocol/Lighting 2.5]]. Untested on hardware (inferred).
- `hid_usage` — key names ↔ USB-HID keyboard usage IDs and modifier bits (public HID spec).
- `event` — hardware-event decoder (records 5/9). Implements [[Protocol/Hardware Events]].

Single source of truth: other crates never build report bytes themselves.

Vault: [[Architecture/Crates]]
