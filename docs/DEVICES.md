# HyperX headsets: support catalog

HyperX is replacing **NGENUITY Legacy (2020–2025)** with a new **NGENUITY (2025)**. According to HyperX, the
new app supports only five headsets, while Legacy — the only official software for every other model — gets
"technical support and maintenance for a limited time". This page tracks which headsets this project
supports or could support, so they keep working after Legacy is gone.

Status legend:

- **Supported** — implemented and tested on real hardware.
- **Experimental** — implemented but not yet confirmed by someone who owns the device.
- **Researched** — the protocol is known, not implemented yet.

## Headsets

| Headset | New NGENUITY (2025) | Legacy only | Protocol family | USB ID | Status | Issue |
|---|:-:|:-:|---|---|---|---|
| **Cloud Alpha S** | | ✔ | Classic DSP | `0951:16ED`, `03F0:0C91` | **Supported** (firmware `0x4110`) | [#8](../../../issues/8) |
| Cloud Flight S | | ✔ | Classic DSP | `0951:16EA` | **Experimental** (7.1, EQ, mic monitoring, battery) | [#1](../../../issues/1) |
| Cloud II Wireless | | ✔ | Classic DSP | `0951:1718`, `03F0:0C8A` | **Experimental** (7.1, EQ, mic monitoring, mic, battery) | [#1](../../../issues/1) |
| Cloud Alpha Wireless | | ✔ | Wireless control set | `0951:1743`, `0951:1765`, `03F0:098D` | **Experimental** (battery, mic, mic monitoring and level, auto power-off, voice prompts) | [#2](../../../issues/2) |
| Cloud II Wireless DTS | | ✔ | Wireless control set | `03F0:018B`, `03F0:0696`, `03F0:038B`, `03F0:058B` | **Experimental** (battery, mic, mic monitoring, auto power-off) | [#2](../../../issues/2) |
| Cloud II Core Wireless | | ✔ | Wireless control set (`66`) | `03F0:0995`, `03F0:069F` ² | **Experimental** (battery, mic, mic monitoring and level, auto power-off) | [#3](../../../issues/3) |
| Cloud Stinger 2 Wireless | | ✔ | Wireless control set (DTS) | `03F0:0D93` | **Experimental** (battery, mic monitoring, auto power-off) | [#3](../../../issues/3) |
| Cloud III Wireless | ✔ | | Wireless control set (`66`) | `03F0:05B7`, `03F0:0C9D`, `03F0:06B7`, `03F0:0E9D` | **Experimental** (battery, mic, mic monitoring and level, auto power-off) | [#3](../../../issues/3) |
| Cloud III (wired) | ✔ | | Feature `20` | `03F0:089D`, `03F0:03CC`, `03F0:0ACE` | **Experimental** (mic monitoring and level) | [#3](../../../issues/3) |
| Cloud MIX 2 | | ✔ | MIX | `03F0:0FAE` | Researched | [#4](../../../issues/4) |
| Cloud MIX Buds | | ✔ | MIX | `03F0:078A` | Researched | [#4](../../../issues/4) |
| Cloud MIX Buds 2 | | ✔ | MIX | `03F0:039E` | Researched | [#4](../../../issues/4) |
| Cloud III S Wireless | ✔ | | Universal | `03F0:06BE`, `03F0:05BE`, `03F0:01CC`, `03F0:02CC` | Researched | [#5](../../../issues/5) |
| Cloud Flight 2 Wireless | ✔ | | Universal | `03F0:0AC1`, `03F0:0BC1`, `03F0:06C1`, `03F0:0CC1` | Researched | [#5](../../../issues/5) |
| Cloud Alpha 2 Wireless | ✔ | | Universal? | `03F0:0ABE`, `03F0:0BBE`, `03F0:08BE` | Not started | — |
| Cloud Flight Wireless | | ✔ | Basic | `0951:16C4`, `0951:1723`, `03F0:0E90` | **Experimental** (battery, headset off) | [#6](../../../issues/6) |
| Cloud Stinger Core Wireless (+ 7.1) | | ✔ | Basic | `0951:170B` | **Experimental** (battery, headset off) | [#6](../../../issues/6) |
| Cloud Stinger S | | ✔ | — | `0951:1715` | Nothing to control over USB ³ | [#6](../../../issues/6) |
| Cloud Stinger Core + 7.1 | | ✔ | — | `0951:1709` | Nothing to control over USB ³ | [#6](../../../issues/6) |

Most models have one ID from the Kingston era (`0951`) and one or more from the HP era (`03F0`), and wireless
models usually have one ID for the headset and another for the dongle.

## Protocol families

Models with the same command set almost certainly share a protocol and are implemented together.

| Family | What it controls | Notes |
|---|---|---|
| **Classic DSP** | 7.1 virtual surround, 10-band parametric EQ, sidetone, battery (wireless models) | Same DSP commands as the Cloud Alpha S, documented in [PROTOCOL.md](PROTOCOL.md) |
| **Wireless control set** | Battery and charging, auto shutdown, mic mute, sidetone on/off and volume, voice prompts, connection state | Same commands on several models, with different headers (see [PROTOCOL.md](PROTOCOL.md)); the Cloud III Wireless / Cloud II Core Wireless use their own `66` set |
| **Feature `20`** | Mic monitoring | Cloud III (wired) |
| **MIX** | Active noise cancellation and level, EQ preset, low-latency mode, battery per earbud and case | |
| **Universal** | Generic audio settings, onboard EQ, battery, sleep time, 3-in-1 dongle | The protocol of newer HyperX devices (also mice and keyboards) |
| **Basic** | Battery and connection state | Few or no controls; some wired models don't need an app at all |

## USB IDs and control interfaces

The control interface is written as in the Windows HID path: `MI_xx` is the USB interface number and
`Colxx` the top-level HID collection inside it. Report IDs and commands are documented per family in
[PROTOCOL.md](PROTOCOL.md) as each one is implemented.

| Headset | USB IDs | Control interface | Cross-checked |
|---|---|---|---|
| Cloud Alpha S | `0951:16ED`, `03F0:0C91` | `MI_05` | **Tested** (`16ED`), HeadsetControl (`16ED`) |
| Cloud Flight S | `0951:16EA` | `MI_05&Col03` | — |
| Cloud II Wireless | `0951:1718`, `03F0:0C8A` | `MI_03&Col03` | HeadsetControl (`1718`) |
| Cloud II Wireless DTS | `03F0:018B`, `03F0:0696` | `MI_03&Col03` | HeadsetControl ¹ |
| | `03F0:038B`, `03F0:058B` | `Col02` (no interface number) | — |
| Cloud Alpha Wireless | `0951:1743`, `0951:1765`, `03F0:098D` | `MI_03&Col01` | HeadsetControl (`098D`) |
| Cloud II Core Wireless | `03F0:0995`, `03F0:069F` ² | `MI_03&Col02` | — |
| Cloud Stinger 2 Wireless | `03F0:0D93` | `MI_03&Col03` | — |
| Cloud III (wired) | `03F0:089D`, `03F0:03CC`, `03F0:0ACE` | `MI_03`, usage page `0x01C0`, usage 1 (`Col02` on `089D`) | HeadsetControl (`089D`, `03CC`) |
| Cloud III Wireless | `03F0:05B7`, `03F0:0C9D` | `MI_03&Col01` | — |
| | `03F0:06B7`, `03F0:0E9D` | whole device | — |
| Cloud MIX Buds | `03F0:078A` | `MI_03&Col04` (+ `Col05` notifications) | — |
| Cloud MIX Buds 2 | `03F0:039E` | `MI_03&Col05` | — |
| Cloud MIX 2 | `03F0:0FAE` | `MI_03&Col06` | — |
| Cloud III S Wireless | `03F0:06BE` | `MI_03&Col05` | — |
| Cloud Flight 2 Wireless | `03F0:0AC1` | `MI_03&Col04` (Universal protocol) | — |
| Cloud Alpha 2 Wireless | `03F0:0ABE`, `03F0:0BBE`, `03F0:08BE` | — | HeadsetControl (`0ABE`, `08BE`) |
| Cloud Flight Wireless | `0951:16C4`, `0951:1723`, `03F0:0E90` | `MI_03&Col03` | HeadsetControl (`16C4`, `1723`) |
| Cloud Stinger Core Wireless (+ 7.1) | `0951:170B` | `MI_03&Col03` | — |
| Cloud Stinger S | `0951:1715` | `MI_03&Col02` | — |
| Cloud Stinger Core + 7.1 | `0951:1709` | `MI_03&Col02` | — |

**Cross-checked** lists where an ID or protocol also appears outside this project:
[HeadsetControl](https://github.com/Sapd/HeadsetControl) device definitions, or a test on real hardware.

Notes:

1. HeadsetControl calls `03F0:018B` / `03F0:0696` "Cloud II Wireless"; here they use the DTS protocol.
   They are probably HP-era revisions of the same headset; a tester's report will tell.
2. `03F0:069F` looks like another edition of the Cloud II Core Wireless (neighbouring ID, same interface);
   its retail name is unknown. If you own it, please send a [device report](../../../issues/new?template=device-report.yml).
3. These two wired models have nothing to control over USB: their 7.1, EQ and reverb are processed on the PC
   by a Windows audio driver, not by the headset, so there is nothing for Nogenuity to drive.

## Out of scope

- **Firmware updates.** Risk of bricking a headset, and the firmware belongs to HP. Never part of this project.
- **DTS Headphone:X and SONAMI spatial audio.** Licensed audio processing; the app may toggle it where the
  headset exposes a switch, but does not reimplement it.
- **Mice, keyboards, microphones, webcams.** Possibly later; headsets first.

## How to help

If you own one of these headsets, you can make it supported: see [Testing a headset](../CONTRIBUTING.md#testing-a-headset)
and the [Device report form](../../../issues/new?template=device-report.yml). Testing never changes your
headset's firmware.

## Adding a headset (for developers)

Each protocol family is one module under `src/devices/`. Adding a model needs no UI code for
capabilities the app already shows:

1. Write the module (`src/devices/alpha_s.rs` is the reference): a `pub const MODEL: Model` with the
   USB IDs, the HID interface of the control collection, the report size and its `Caps`, and a type that
   implements the `Headset` trait. Only implement what the headset has; the rest of the trait fails by
   default and the UI never calls it.
2. Add it to `MODELS` in `src/devices/mod.rs`. Leave `tested: false` until someone has confirmed it on
   real hardware: the window then shows an **EXPERIMENTAL** badge.
3. The cards follow the capabilities (`src/layout.rs`, unit-tested): no 7.1 means no 7.1 switch, no EQ
   means no equalizer card and a shorter window, and so on.

`src/hid.rs` has the shared helpers learned on the Alpha S: wait for each reply (`read_ack`), recover a
box that rejects writes because a reply was left unread (`send`), a cross-process lock and `hid.log`.

To try the UI without a headset, start the app with `NOGENUITY_DEMO=1` (a simulated headset with every
capability), `NOGENUITY_DEMO=basic` (microphone and battery only) or `NOGENUITY_DEMO=wireless` (microphone,
battery and the headset card, like a wireless model). Demo settings go to
`settings-demo.txt` and never touch the real ones.
