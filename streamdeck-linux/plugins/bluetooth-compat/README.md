# Bluetooth for Linux

Independent BlueZ implementation for Deckforge. No Windows helper, Wine, vendor
JavaScript, or vendor artwork is shipped. The original Bluetooth Buddy action
identifiers are retained so saved profiles keep working. This is a Deckforge
replacement, not an official FlowKits release.

## Actions

- **Bluetooth Device:** connect/disconnect a paired device. Settings retain
  `address`, `deviceName`, `customLabel`, `showLabel`, and `showBattery`.
- **Bluetooth Radio:** toggle adapter power. With multiple adapters, an enabled
  adapter makes the next toggle turn all adapters off.
- **Disconnect All:** disconnect connected paired devices.
- **Bluetooth Battery:** rotate or tap to browse paired devices; press the dial
  to connect/disconnect. `pinnedAddress` locks the display to one device.

Battery percentages are shown only when BlueZ exposes `org.bluez.Battery1`.
Missing values are labelled unavailable. Devices must first be paired through
Linux Bluetooth settings. Adapter/device errors appear in the settings panel
and produce a key alert. The new artwork uses Deckforge's colors; the vendor's
custom color editor and animations are not implemented.

## Requirements and install

Requires Node.js 24, Python 3, `busctl`, a running BlueZ system service, and the
user session's existing permission to control Bluetooth. There are no npm or
pip dependencies. Reads poll BlueZ every two seconds; commands use its D-Bus
methods directly without a shell.

From the repository root:

```sh
python scripts/install-bluetooth-compat.py
```

The installer verifies the known original package or a previously installed
replacement, stages the new files, and moves the previous whole folder into
Deckforge's `plugin-backups` directory. It prints the exact backup path. Restart
the plugin from Deckforge's plugin manager after installing. Saved profile
settings live in Deckforge and are retained. Marketplace updates may replace
this independent implementation; rerun the installer only for a supported
original version.

To roll back, disable the plugin, move the current folder aside, restore the
printed backup to its original `com.flowkits.bluetooth.sdPlugin` location, and
enable the plugin again. Preserve both copies until the restored version has
been checked.

## Verification

```sh
python tests/bluetooth_compat_test.py
python tests/bluetooth_plugin_interop.py
node tests/bluetooth_native_pi.cjs /path/to/deckforge
```

The helper tests cover BlueZ inventory, device removal, command errors, radio
power, and backup handling. The integration fixture drives the real Deckforge
host and plugin with simulated Bluetooth devices. Native inspector tests use
the actual WebKit helper for all four settings panels. These fixtures do not
prove a physical device accepted a connect/disconnect request.
