# Razer HID protocol — overview

Synapse 4 implements several protocol generations (module names in `background-manager`):
- **Protocol 2.5** — `rzDevice25*` (classic 90-byte feature report; most mice/keyboards)
- **Protocol 3.0** — `rzDevice30`
- **Protocol 4.0** — event parser `rzHardwareEventsProtocol40Parser` (newer devices/earbuds)

Detailed notes (filled by RE passes):
- [[Protocol/Report Format 2.5]]
- [[Protocol/Commands 2.5]]
- [[Protocol/Protocol 3.0 and 4.0]]
- [[Protocol/Hardware Events]]
- [[Protocol/Windows Transport]]

Status: ✅ RE pass 1 done (2026-11-09). Viper V3 HyperSpeed uses **2.5**. Open questions: [[Status]].
