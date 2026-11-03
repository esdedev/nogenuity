# HyperX Cloud Alpha S — control box HID protocol

The Cloud Alpha S sections were verified on a real headset (USB `0951:16ED`, firmware `0x4110`) on
Windows 11, byte by byte. The other families are marked **Not verified on hardware**: they are the starting
point for a tester's log, not settled fact.

This document describes device behaviour only, for interoperability. No vendor code is included in this
project.

## Transport

- The control box is a USB composite device. Audio uses interfaces 0 (Game) and 2 (Chat); the control
  protocol uses a vendor HID collection on **interface 5**, **usage page `0xFF13`, usage `0x0001`**.
- Every command is a **62-byte output report with report ID `0x06`**. The reply is fetched with
  **GET_REPORT (input report) for the same ID** (`HidD_GetInputReport` on Windows).
- The device acknowledges by echoing the request header with **bit 7 of byte 5 set** (`0x80`).
- On **Linux** there is one `/dev/hidrawN` per interface: pick the one whose `uevent` has
  `HID_ID=0003:00000951:000016ED` and `HID_PHYS=…/input5`. Write the report with `write()` and fetch the
  reply with the `HIDIOCGINPUT(len)` ioctl (kernel 5.11+); button reports arrive with `read()`.
- Multi-byte integers are **little-endian**. EQ values use **Q8.23 fixed point** (`round(value * 2^23)`
  as a signed 32-bit integer). Levels in the status reply are **Q8.8, big-endian**.

## Control commands (module `68 4A 8E`)

| Byte | Value |
|---|---|
| 0 | `0x06` (report ID) |
| 2 | command kind (`0x01` most commands, `0x0D` for 7.1) |
| 4 | `0x9B` |
| 7–9 | `68 4A 8E` |
| 10 | parameter |

| Parameter | Kind | Meaning |
|---|---|---|
| `0x09` | `0x01` | Read status |
| `0x07` | `0x01` | **Toggle** sidetone (mic monitoring) |
| `0x02` | `0x0D` | **Toggle** 7.1 virtual surround |

**The sidetone and 7.1 commands toggle**, exactly like the 7.1 button on the control box. Byte 5 is
ignored (checked against the box LED: `0,0,1,1,0` toggled the state five times). To *set* a state, read
the status first and only send the command when it differs.

### Status reply

| Byte | Meaning |
|---|---|
| 12, bit 1 (`& 0x02`) | 7.1 surround on |
| 14, bit 0 (`& 0x01`) | sidetone on |
| 14, bit 1 (`& 0x02`) | microphone muted |
| 16–17 | game attenuation in dB, Q8.8 big-endian (0 dB = full volume) |
| 20–21 | chat attenuation in dB, Q8.8 big-endian |

### Asynchronous report `0x0A`

Sent on the interrupt endpoint whenever a control box button is pressed (read it with `ReadFile`):
byte 2 bit 1 = 7.1 on, byte 4 = mute flags, bytes 6–7 / 10–11 = game / chat dB (Q8.8 big-endian).

On Windows, keep a **separate handle** for the blocking `ReadFile`: a pending synchronous read on a
handle blocks every other request on it.

## Equalizer (module `30 2B AE`)

One packet per band and channel (send it twice, with byte 6 = `4` and `5`):

| Byte | Value |
|---|---|
| 0 | `0x06` |
| 2 | `0x0D` |
| 4 | `0x41` |
| 6 | `4` + channel (0 or 1) |
| 7–9 | `30 2B AE` |
| 10–13 | band index (0–9), int32 LE |
| 14–17 | filter type, int32 LE (`0` peaking, `1` low shelf) |
| 18–21 | frequency in Hz, int32 LE |
| 22–25 | Q, Q8.23 |
| 26–29 | gain in dB, Q8.23 |

- **10 bands at most** (band index 0–9).
- **The EQ is only audible while 7.1 is on**: it runs inside the surround DSP.
- Nogenuity keeps gain within **±9 dB** and Q within **0.2 … 5**; wider values have not been tried.
- The EQ is lost when the headset is unplugged; resend it on connect.

## The box blocks while a reply is unread

**The control box does not accept a new command while the reply to the previous one has not been
read.** Every output report is rejected (`ERROR_GEN_FAILURE`, 31) while audio keeps playing, which looks
like a hang. It is not: **a single GET_REPORT for report `0x06` returns the pending reply and unblocks
it**, no unplugging needed.

This happens when the reply is read before the device has produced it (so the real reply stays
queued), when a program is killed between sending and reading, or when two programs interleave
requests. To be robust:

1. After each command, poll GET_REPORT until the reply echoes the request (same byte 4 and byte 6,
   bit 7 of byte 5 set) instead of reading once after a fixed delay.
2. If a send is rejected, read once and retry: that clears whatever reply was left behind.
3. Serialize requests across processes (Nogenuity uses a named mutex, `Local\Nogenuity-HID`)
   and do not run another program that controls the headset at the same time.
4. After toggling 7.1 the DSP restarts and takes over a second to answer EQ packets; waiting about
   1.5 s before sending EQ avoids long retries.

## Classic DSP family: Cloud Flight S and Cloud II Wireless

**Not verified on hardware.** On the Alpha S some details only showed up with the headset at hand (see the
sidetone bit below), so treat this as the starting point for a tester's log, not as fact.

| | Cloud Alpha S | Cloud Flight S | Cloud II Wireless |
|---|---|---|---|
| USB ID / control interface | `0951:16ED`, interface 5 | `0951:16EA`, interface 5 (`Col03`) | `0951:1718` (dongle), `03F0:0C8A`; interface 3 (`Col03`) |
| Report | `0x06`, 62 bytes | `0x06` (size not confirmed) | `0x06` (size not confirmed) |
| Status request | kind `0x01`, byte 4 `0x9B`, param `0x09` | `06 00 00 00 FF 00 00 68 4A 8E` (wait 50 ms, then GET_REPORT) | same as Flight S, wait 200 ms |
| Status reply | see above | byte 12 bit 1 = 7.1; byte 14 bit 4 = sidetone ¹; bytes 16/20 = game/chat Q8.8 | as Flight S, plus byte 14 bit 1 = mic muted |
| 7.1 | param `0x02`, kind `0x0D`, **toggles** | kind `0x0D`, byte 4 `0x9B`, byte 5 `1`: param `0x01` with byte 14 = on/off, then param `0x02`, then `0x08` (50 ms apart, each followed by GET_REPORT) | same as Flight S |
| Sidetone | param `0x07`, **toggles** | sub-command `0x19`, byte 16 = on/off (**sets**) | same |
| EQ | byte 6 = `4` + channel | byte 6 = `10` + channel, no reply read, 50 ms between packets | byte 6 = `10` + channel, reply read after each packet |

¹ Bit 4 is the expected position, but on the Alpha S the sidetone turned out to be bit 0. One of the first
things to check in a Flight S / Cloud II Wireless log.

### Second command set (wireless models)

`06 00 02 00 9A 00 00 68 4A 8E 0A 00 00 00 BB <sub> <value>`, i.e. kind `0x02`, byte 4 `0x9A`, byte 10
`0x0A`, byte 14 `0xBB`, byte 15 = sub-command, byte 16 = value:

| Sub | Meaning | Models |
|---|---|---|
| `0x01` | Wireless / connection state | Cloud II Wireless |
| `0x02` | Battery request | both |
| `0x03` | Charging status request | both |
| `0x0C` | Sidetone volume (byte 16) | Flight S |
| `0x13` | Enable sound (byte 16) | Flight S |
| `0x18` | Set auto power-off (byte 16) | Cloud II Wireless |
| `0x19` | Set sidetone on/off (byte 16) | both |
| `0x1A` | Auto power-off request | Cloud II Wireless |

The answers do not come back with GET_REPORT: they arrive on the **interrupt endpoint** (where the Alpha S
sends its button reports). Expected format on the Flight S (byte 2 = `0xBB`, byte 3 = sub-command):

| Report | Meaning |
|---|---|
| `0x0A …` | Same as the Alpha S button report: byte 2 bit 1 = 7.1, bytes 6/10 = game/chat Q8.8 |
| `… BB 01 xx` | byte 4 = 1: headset connected to the dongle, otherwise asleep |
| `0B … BB 02 …` | Battery: **byte 7 = percentage** |
| `… BB 03 xx` | Charging: byte 4 = 0 not charging, 1 charging, 2 / 3 two further states (full / error?) |
| `… BB 04 …` | Multi-function button events (bytes 4 – 8) |
| `… BB 08 / 09 xx` | A mute state (byte 4 = 1 means unmuted) |
| `… BB 11 …` | Firmware version digits (bytes 4 – 7) |
| `… BB 1D xx` | Microphone mute (byte 4 = 1 means unmuted) |

The Cloud II Wireless is expected to send the same `BB 02` / `BB 03` reports.

Nogenuity reads **battery and charging** this way: it sends sub-command `0x02` (then `0x03`), waits
50 ms (the Cloud II Wireless also does a GET_REPORT it discards) and waits up to 1 s for the matching
`BB 02` / `BB 03` report on the interrupt endpoint, read through a second handle as for the wireless
control set. Charging means byte 4 = 1. Both are asked at most every 30 s, also when the headset doesn't
answer. The other reports are not used yet; a tester's `hid.log` is the way to confirm the format.

## Wireless control set: Cloud Alpha Wireless, Cloud II Wireless DTS and relatives

**Not verified on hardware.** Cross-checked with [HeadsetControl](https://github.com/Sapd/HeadsetControl),
whose implementations of both models agree on every command they share.

Both send a request `<header> <command> <value>` and the headset answers with the same header and
command, **on the interrupt endpoint** (a plain read, not GET_REPORT). Nogenuity doesn't wait for an
answer to requests that only set something (HeadsetControl does read one for the DTS auto shutdown,
`22`, which echoes the minutes). The headers differ, so it isn't one protocol:

| | Cloud Alpha Wireless | Cloud II Wireless DTS |
|---|---|---|
| USB IDs | `0951:1743`, `0951:1765`, `03F0:098D` (all USB dongles) | `03F0:018B` (dongle), `03F0:0696`; `03F0:038B`, `03F0:058B` (single interface) |
| Control interface | 3 (`Col01`) | 3 (`Col03`); interface 0 on the single-interface IDs |
| Report size | 31 bytes | 52 bytes out, 20 bytes back |
| Header | `21 BB` | `06 FF BB` |
| Command / value byte | 2 / 3 | 3 / 4 |

| Command | Alpha Wireless | Cloud II DTS | Answer |
|---|---|---|---|
| Connection state | `03` | `01` | Alpha: value 2 = headset on; DTS: 1 or 4 = on |
| Battery | `0B` | `02` | % in byte 3 (Alpha) / byte 7 (DTS) |
| Charging | `0C` | `03` | value 1 = charging |
| Sidetone on? | `05` | `06` | value 1 = on |
| Sidetone level? | `06` | `0B` | |
| Auto shutdown (minutes) | `07` | `07` | value |
| Mic boom plugged | `08` | `08` | value 1 = plugged |
| Voice prompts on? | `09` | — | value 1 = on |
| Mic muted? | `0A` | `05` | value 1 = muted |
| Set sidetone | `10` + 0/1 | `21` + 0/1 | |
| Set sidetone level | `11` + level ¹ | `23` + level ¹ | |
| Set auto shutdown | `12` + minutes | `22` + minutes | |
| Set voice prompts | `13` + 0/1 | — ² | |
| Set mic mute | `15` + 0/1 | `20` + 0/1 | |

¹ The level goes in the value byte, as in HeadsetControl.
² No known voice prompts command on the DTS.

What Nogenuity uses from the second half of the table (the HEADSET card):

- **Auto power-off**, both models: get / set with the minutes in the value byte, 0 = never. Both take any
  number of minutes (HeadsetControl allows 0-255); the app offers Never, 10, 20, 30 and 60.
- **Mic monitoring level**, Alpha Wireless only: `11` + level, **0-128** as in HeadsetControl, which sends
  it right after turning sidetone on with `10 01`. Here on/off stays with its own switch; turning it on
  sends the level again, in case it was changed while off. The level is read back with `06` (value
  byte), ignored if above 128; the format of that answer is not confirmed.
- **Voice prompts**, Alpha Wireless only: `09` / `13`.
- Not on the DTS: the range of its level command (`23`) is unknown and HeadsetControl doesn't implement
  it; voice prompts, see note ².

These values almost always change from the app, so Nogenuity reads them once a minute instead of on every
status poll, which would wait up to 1 s per unanswered command; a change made on the headset itself
shows up within a minute. A value is only sent when it differs from the headset's, and the app keeps
what it sent rather than reading it straight back. Like 7.1 and mic monitoring, they are saved and
re-applied when the headset connects or is switched on.

The same reports also arrive unasked when something changes on the headset (mute switch, sidetone
button, power): Nogenuity skips them while waiting for an answer and picks the new state up in its next
status poll.

Only the dongle answers the connection state (on the DTS, `0x018B`; every Alpha Wireless ID is a dongle).
With the headset off, Nogenuity shows **HEADSET OFF** instead of "disconnected".

### Cloud Stinger 2 Wireless

`03F0:0D93`, `MI_03&Col03`: byte for byte the DTS protocol above (same header and command numbers).
Nogenuity uses connection state `01`, battery `02`, charging `03` (byte 4: 1 charging, 2 full, 3 error),
sidetone `06` / `21` and auto shutdown `07` / `22`. The mic state (`05`) and the sidetone level (`23` /
`0B`) are not known to work on this model, so Nogenuity doesn't show them.

### Cloud III Wireless and Cloud II Core Wireless: the `66` set

**Not verified on hardware.** HeadsetControl supports neither.

| | Cloud III Wireless | Cloud II Core Wireless |
|---|---|---|
| USB IDs | `03F0:0C9D`, `03F0:05B7` (dongles, `MI_03&Col01`); `03F0:0E9D`, `03F0:06B7` (whole device) | `03F0:0995`, `03F0:069F`, `MI_03&Col02` |
| Report | output `66`, 62 bytes | same |

Requests are `66 <command> <value>`; read requests have bit 7 set and the answer comes on the interrupt
endpoint as `66 <same command> <value> …`. The headset also sends `66 09`-`0D` on its own (sidetone, mic
mute, headset connected, charging, battery). Nogenuity waits 10 ms after each write and up to 1 s for an
answer.

| Command | Read | Set | Answer |
|---|---|---|---|
| Headset on (dongle state) | `82` | | value 1 = on |
| Sidetone | `84` | `01` + 0/1 | value 1 = on |
| Auto power-off | `85` | `02` + minutes | minutes, 0 = never; Nogenuity offers Never, 10, 20, 30 |
| Mic muted | `86` | `03` + 0/1 | value 1 = muted |
| Playback muted | `87` | `04` + 0/1 | not used |
| Sidetone level | `88` | `05` + level | **-5 … +5**, signed byte (`FB` … `05`), 0 by default |
| Battery | `89` | | % in byte 4; bytes 2-3 (probably the voltage) both zero = no reading |
| Charging | `8A` | | value 1 = charging |

Nogenuity offers the sidetone level on both models: its slider maps 0 … 10 to -5 … +5 and shows the
device's value (0 is the factory level). It is less certain on the Cloud III Wireless. The level, auto power-off and voice prompts saved for one model are only re-applied to that model. Also there but not used: noise gate `0F` / `8D`
and mic boom `8C` (Core), colour `8F` and the pairing key `83` / `1E` on the `05B7` dongle (Cloud III
Wireless).

## Cloud III (wired): feature report `20`

**Not verified on hardware.** `03F0:089D` (`MI_03&Col02`), `03F0:03CC` and `03F0:0ACE`, usage page `0x01C0`,
usage 1. The page is below the usual `0xFF00` vendor range, so on Windows it has to be asked for by value. That
`0ACE` has the same interface and commands as the other two is an assumption.

Mic monitoring and its level: SET_FEATURE `20 <command> <value>`; for a read Nogenuity waits 500 ms and does
GET_FEATURE `20`, value in byte 2. Read sidetone `06` (1 = on), read gain `05`; set sidetone `86` + 0/1, set gain
`85` + gain. Valid gains are `C4` … `FA` (probably -60 … -6 dB); the level slider covers that range.

Turning mic monitoring on, Nogenuity first reads the gain and, only if it's out of range, sets `85 F0` and saves it
with `87 01`. HeadsetControl just sends `20 86 01`. A level set from the slider isn't saved on the headset: the
app sets it again on connect.

## Basic models: Cloud Flight Wireless and Cloud Stinger Core Wireless

**Not verified on hardware.** They only report battery, charging and whether the headset is on (the USB
dongle stays plugged in); neither has controls Nogenuity can show.

**Cloud Flight Wireless** (`0951:16C4`, `0951:1723`, `03F0:0E90`, interface 3, `Col03`):

- Request: output report `21 FF 05` + zeros (20 bytes, as in HeadsetControl and a USB capture).
- Reply: on the interrupt endpoint, starting `21 FF 05`; bytes 3-4 are the battery voltage in mV,
  big-endian. Nogenuity counts the headset as off when nothing arrives within 2 s.
- Charging: voltage `0x1014`-`0x11FF`. Otherwise Nogenuity turns the voltage into a percentage with an
  approximate lithium cell curve (see below). HeadsetControl uses a polynomial instead.
- Mic mute and power events come on another collection according to community projects; Nogenuity doesn't
  read them yet.

**Cloud Stinger Core Wireless** (`0951:170B`, the dongle, interface 3, `Col03`):

- Feature reports, ID `FF`, both ways: SET_FEATURE with the request (padded to the descriptor's feature
  size), wait 20 ms, GET_FEATURE `FF`. Offsets below include the ID byte.
- Headset on: request `FF 07 00 FD 04 00 00 02 82 03 05 29 0F 00 00 03 68 10 00 00`, reply byte 11 = 1.
- Battery: request `FF 07 00 FD 04 0C F1 02 01 04 F0 0C` (byte 6 `F1` = dongle). Reply byte 9 = charge
  state (`03` not charging, `05` charging, `06` full; other values are ignored), bytes 11-12 = voltage in
  mV, little-endian.
- The reading bounces for a few seconds when the cable is pulled, so leaving a charging state is only
  taken after four equal readings in a row, keeping the previous battery meanwhile.
- On Linux, hidraw doesn't give the feature size: Nogenuity reads it from the HID report descriptor (the
  size of report `FF`).
- On Windows both models are opened on `Col03` (`collection` in the model).

**Battery percentage** (both models): Nogenuity interpolates the voltage on an approximate 11-point curve for
a 3.7 V lithium cell (0 %, 10 %, … 100 %), one for discharging and one for charging, and truncates. These
are generic values, not measured on these headsets: a tester's log with voltage and the headset's own
voice prompt or LED level is the way to tune them.

## Game / chat balance

There is no known command to set the balance on this model; the balance is done by the box's buttons.
This app offers a software balance by changing the Windows volume of the **"HyperX Cloud Alpha S Game"**
and **"… Chat"** output devices.

## Toggle commands

On the Alpha S the 7.1 and sidetone commands toggle instead of setting a value. A program that sends them
without reading the state first can drift out of sync with the hardware, so Nogenuity always reads the status
before sending them (see [Control commands](#control-commands-module-68-4a-8e)).
