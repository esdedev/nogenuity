# Contributing to Nogenuity

## Testing a headset

Most HyperX headsets in [docs/DEVICES.md](docs/DEVICES.md) are implemented but have never been tried with
Nogenuity on real hardware. Owning one is the most useful thing you can bring.

1. Install Nogenuity ([README](README.md)) and close NGENUITY: two programs talking to the headset at once
   confuse it.
2. Plug the headset in. Models not yet confirmed show an **EXPERIMENTAL** badge.
3. Try each control in the window, and change the same settings on the headset itself to see if the window
   follows.
4. Click **Send report** at the bottom of the window. Nogenuity saves `nogenuity-report.txt` in its data folder,
   opens that folder and opens the [Device report form](../../issues/new?template=device-report.yml). Fill it
   in and drag the file into it.

### Is it safe?

- **Nogenuity never updates firmware**, and never will: that's out of scope for this project.
- It only sends the commands documented for that model in [PROTOCOL.md](docs/PROTOCOL.md).
- The worst case seen so far is the control box ignoring commands until you unplug it and plug it back in.
  If that happens, say so in the report: the log shows what was sent just before.
- The report contains the app version, OS, USB ID, firmware version and the HID log (raw bytes sent to and
  received from the headset). No names, accounts or files.

## Adding a headset

See [Adding a headset](docs/DEVICES.md#adding-a-headset-for-developers) and [PROTOCOL.md](docs/PROTOCOL.md).
Work happens in a branch and a pull request per issue; every model stays experimental until someone who owns
it confirms it.

### Ground rules

- **Behaviour only.** Protocol docs and code comments describe what the device does: report IDs, bytes,
  timings, USB IDs. Never include code, class or function names, internal codenames, assets or data tables
  copied from vendor software.
- **Your own data.** EQ presets, battery curves and similar values are written or measured by contributors,
  not copied.
- **Code comments in English.**

## Building

See [Build](README.md#build). Run `cargo test` before opening a pull request; on Windows and Linux the build
should have no warnings.
