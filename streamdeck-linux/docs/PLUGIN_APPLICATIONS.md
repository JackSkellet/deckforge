# Plugin application monitoring

Open **Plugins → Monitored applications** for a plugin that declares
`ApplicationsToMonitor`. Enter a Linux executable basename, such as `obs`, or an
absolute executable path for each identifier it needs. Choose **Apply mappings**.
Leaving a field blank disables that identifier. These settings belong to the
plugin installation, survive restarts and do not change exported profiles.

Windows and macOS declarations start unmapped. Deckforge does not guess that
`Program.exe` means a Linux program named `Program`. The optional Deckforge
`ApplicationsToMonitor.linux` manifest extension defaults to the declared Linux
name or absolute path and can also be overridden or disabled. Executable names
and paths match case-sensitively. Paths refer to the executable reported by
`/proc/PID/exe`, including resolved symlinks. Arguments, shell expressions,
desktop file IDs and window titles are not interpreted.

For example, mapping `obs64.exe` to `obs` sends the original logical identifier
to the unchanged plugin:

```json
{"event":"applicationDidLaunch","payload":{"application":"obs64.exe"}}
```

Only interested plugins receive application events. No action context or device
is attached. Already-running matching processes replay when the plugin connects;
each process produces a launch and a termination event. Two instances of one
executable therefore produce two launch events. If mappings overlap, the first
matching declaration wins for that process, in Windows, macOS, then Linux
declaration order. Applying changed mappings ends old subscriptions and reports
currently matching processes under the new mappings; it does not stop or start
applications.

## Linux observation

A Rust worker reads current-user process directory ownership, start time and
executable identity from `/proc`. It reads no process command lines or environment
variables, changes no focus, and launches or signals no applications. It starts
only when a connected plugin has a nonempty mapping. It samples every 500 ms, so
very short-lived processes may be missed. This is not a process audit log.

PID and kernel start time distinguish reused process IDs. Unreadable processes
retain their last known identity while still present; unrelated readable
processes continue producing events. A failed whole scan preserves the preceding
inventory and reports an error. Zombies and disappeared processes are removed.
The inventory is held in memory and is not exposed in editor snapshots or saved
to session files. The kernel's ` (deleted)` path suffix is removed only when
followed executable metadata confirms that the inode has no remaining links.
This preserves the identity of a running application during ordinary replacement
and distinguishes a real filename ending in that suffix. Deleted paths whose
inode still has another hard link remain a known identity limitation.

## Contract and validation

The documented events are in Elgato's
[system guide](https://docs.elgato.com/streamdeck/sdk/guides/system/) and
[plugin WebSocket reference](https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/).
Static inspection of the pinned Stream Deck 7.6 Windows executable establishes
initial replay on plugin connection, per-process multiplicity, executable basename
with extension and plugin-specific delivery. Its stored-name matcher compares
bytes exactly; upstream manifest normalization and process eligibility rules
remain unproven. Explicit Linux mappings and current-user-only observation are
Deckforge policies. Local records are in `evidence/plugin-application-monitor/`.

`tests/plugin_applications_interop.py` observes temporary owned executable copies
through the unchanged official SDK 2.0.4. It checks existing instances, live
launch/exit, interested-plugin isolation, plugin and host restart replay,
persistence and stale-save rejection. Rust fixtures cover PID reuse, executable
changes, partial failures, scan failure and bounded shutdown. The native editor
fixture checks mapping drafts, explicit Apply, stale reload and window recreation.
These tests do not establish compatibility with every third-party plugin or
monitor processes inside inaccessible container namespaces.
