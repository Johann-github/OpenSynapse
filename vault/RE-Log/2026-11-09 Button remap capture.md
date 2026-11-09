# 2026-11-09 Button remap capture

**Question:** Which data bytes does Synapse send in 02/0C (single button assignment)?
**Evidence (source / tool):** USBPcap + Wireshark 4.x capture of Synapse on the Viper V3
HyperSpeed dongle (PID 0x00B8, USBPcap3, device 3.9), decoded with tshark. The capture file
lives outside the repo (`~/re-work/button-remap-2.pcapng`).

User actions (in order): press mouse button 4 a few times → Synapse: button 4 → "Doppelklick"
→ click a few times → Synapse: back to "Maustaste 4".

## Findings
- (observed) Synapse sends exactly one SET_REPORT (wLength 90, no report ID on the wire) per
  change, then one GET_REPORT ~28 ms later. No 02/8C read before or after.
- (observed) Change to double-click, tid 0x06:
  `00 06 00 00 00 0a 02 0c | 01 04 00 0b 01 01 | 00… | crc 0a`
  → profile 1, buttonId 4, mode 0 (normal), functionId 0x0B (11, double-click), dataLen 1, data `01`.
- (observed) Change back, tid 0x07:
  `00 07 00 00 00 0a 02 0c | 01 04 00 01 01 04 | 00… | crc 05`
  → functionId 1 (mouse button), dataLen 1, data `04` = mouse button 4.
- (observed) data_size is 0x0A (10), not the 80 from the command table. The code now sends 10.
- (observed) Both replies: status 0x02 (success), args echoed.
- (inferred) Double-click data `01` names the button to double-click (1 = left, same as buttonId).
- (observed) Device mode Driver (Synapse running): after the double-click write, the
  interrupt reports on EP1 still showed the raw button-4 bit (`08 …`). The firmware sends
  the raw button; Synapse probably produces the double click itself in this mode. The
  onboard effect without Synapse is untested.

## Open questions
- Does the mapping act in firmware once the device leaves driver mode (Synapse closed)?
- Data layouts of the other function types (key, DPI, macro, …).
- Does the device accept data_size 80 for 02/0C?

## Next
- Read the current assignment with 02/8C via the CLI and compare against this capture.
