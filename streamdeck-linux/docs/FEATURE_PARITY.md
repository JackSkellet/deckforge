# Stream Deck desktop feature parity

Target: a Linux desktop application with a Rust engine and embedded TypeScript editor, with the behavior of Elgato Stream Deck **7.6.0.23012**. Existing Elgato profiles and plugins remain first-class compatibility requirements. A browser editor, a successful sample plugin, or a working USB display does not satisfy the whole target.

This ledger began as the **2026-10-11 baseline** and now includes the first native desktop implementation. Status describes inspected code and recorded evidence, not planned work. Update each row when its acceptance evidence exists. The acceptance tests below are proposed project tests; they are not claims that Elgato publishes those tests.

The official [7.6 release notes](https://help.elgato.com/hc/en-us/articles/51156609278737-Elgato-Stream-Deck-7-6-Release-Notes) date the release to 22 September 2026. They include Neo Infobar actions, new plugin integrations, image drag/drop fixes, automatic launch fixes, Website icon refresh and Network Dock fixes. Thus, “full 7.6 parity” cannot be reduced to eight keys and four dials. The attached Stream Deck+ is the first physical acceptance device; other models and external integrations remain explicit gaps until assessed.

## Current work order

The appearance editor checkpoint is complete. The user's latest order on
12 October 2026 is to support other Stream Deck devices, improve plugin compatibility,
and add Wave Link meters. Sharing and Smart Profiles come at the end. Wave Link
effects and screensaver completion are deferred. The user will perform
long-running reliability testing; it is not part of the agent's current work
queue. Short functional/regression checks continue. GitHub CI remains manual-only.

## Status and evidence rules

| Status | Meaning |
| --- | --- |
| **Verified subset** | A named behavior has a recorded successful test. The rest of its feature remains open. |
| **Partial** | Some implementation exists, with a specific missing behavior or unverified boundary. |
| **Missing** | No implementation found in this source snapshot. |
| **Unresolved** | Required behavior, dependency, service access, or format still needs investigation. This is not a pass or an exclusion. |
| **Retired service** | Official documentation says the upstream service was removed; preserve applicable local-file behavior. |

Evidence levels are separate: **D** official documentation, **A** shipped artifact inspection, **C** local source, **T** automated host test, **H** physical hardware observation, **U** user workflow confirmation. D/A/C never establish T/H/U. A simulator never establishes hardware behavior. A plugin logging an unsupported command never establishes that command's visible effect.

No row below currently establishes complete desktop parity. Native Windows/macOS executables, OS APIs, and service accounts are dependencies to solve or report; they must not disappear from the compatibility inventory because a Node sample works.

## Desktop application and configuration

Desktop interaction requirements come from the official [plugin setup guide][plugins], [profile guide][profiles], [preferences guide][preferences] and [7.6 release][release76]. The standalone native shell is also an explicit user requirement.

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| UI-01 | Standalone native desktop application; launch without a browser | **Verified subset, C/T/H/U.** Default launch opens a native Wry/WebKit window containing the React/TypeScript editor, connected to the in-process Rust controller. Native capture and owned-child shutdown are verified; user-confirmed real-device settings edits refer to the earlier egui editor. The latter remains a temporary explicit fallback. Desktop menu entry validates; separate menu-click acceptance remains unverified. | Launch installed application from Linux desktop menu. Configure and operate the device entirely within its own window. Capture window/process evidence and user acceptance. |
| UI-02 | Device canvas, key/dial selection, live previews and action assignment | **Verified subset, C/T/H/U.** The native canvas uses model geometry for Mini, 15-key, XL, Plus, Plus XL, Neo and Pedal layouts, with real host previews. Other-model physical acceptance remains open. User confirmed native settings/save affects physical input. Replacement, deletion, all-slot geometry and broader keyboard workflows still need end-to-end acceptance. | Native canvas maps every Plus slot accurately; assignment, replacement, deletion, keyboard navigation and persisted state all work on hardware. |
| UI-03 | Action library with categories, search, controller filtering and useful plugin metadata | **Partial, C/T.** Native Actions sidebar searches action/plugin names, UUIDs, categories and tooltips; groups actions by manifest category and filters by controller. Incompatible entries can be shown disabled; duplicate names retain distinct UUIDs. Search/filter drafts survive window recreation. Manifest SVG/raster icons now appear as bounded cached thumbnails; broader keyboard acceptance and exact reference appearance remain open. | Search/filter official manifests; key actions cannot be accidentally placed on dial-only slots; categories, icons and disabled/incompatible states are clear. |
| UI-04 | Drag/drop, move/copy/paste, reorder, remove, context menus and reusable action editing | **Verified subset, C/T.** Native right-click/Edit menus, keyboard copy/cut/paste, canvas moves/swaps and Ctrl-drag copies preserve nested actions, settings and resource bindings. Cuts retain their source until paste; stale targets/read-only/incompatible destinations are rejected. Folder copies receive independent page trees; state assets survive cross-profile export/restart. System clipboard interoperability, action-sharing files and undo remain missing. | Exercise internal and file-manager drag/drop; copy across pages/profiles; verify nested data and settings survive. Test undo/recovery as a Linux usability requirement. |
| UI-05 | Action-specific configuration with plugin property inspectors | **Verified subset, C/T.** Native Plugin settings opens a WebKit inspector. Unchanged Counter PI registers; instrumented original control saves into the real plugin. Broader forms/file controls remain unverified. | Open unchanged official property inspector; edit controls, file selectors and dynamic lists; verify visible plugin/hardware response and restart persistence. [PI guide][pi] |
| UI-06 | Titles, typography, image selection, icon/background libraries, preview fidelity | **Verified subset, C/T.** Per-state title/icon editing, multiline titles, font family/size/style/underline/colour/alignment/visibility, Rust-rendered draft previews and explicit Save/reset are implemented. User overrides take precedence over SDK updates; original image bytes survive export. Font substitutions are reported. Animation, complex text shaping, image cropping/asset libraries and Windows pixel equivalence remain open. | Compare imported title styling, custom icons and state changes against reference screenshots; verify scaled native UI and hardware rendering. [Key guide][keys], [layouts][layouts] |
| UI-07 | Window/tray lifecycle, startup preference, diagnostics and update/about UI | **Partial, C/T.** Closing the editor preserves the host and unsaved drafts; launcher/tray reopening recreates the native window in the same process. Private state-scoped instance activation, explicit Quit, background startup, SIGINT/SIGTERM and owned-child cleanup are tested. Native window generations and plugin heartbeat continuity are checked. Native startup preference and About version/source are implemented; generated XDG entry is validated and launched through GIO with isolated state and reserved-character paths. Actual next-login startup and update UI remain open; compositor focus, real title-bar close input and visible tray painting remain unverified. | Close/reopen window without interrupting configured background actions; explicitly quit stops only this app's children; test login startup, version display and bounded logs. [Preferences][preferences], [7.6 fixes][release76] |

## Profiles, pages, folders and portable data

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| PR-01 | Read existing `.streamDeckProfile` archives/directories without losing unknown data | **Verified subset, A/T.** Six SDK sample profiles and two shipped Plus profiles import; raw fields, settings, states and images are retained. | Expand corpus to user exports from supported app versions and device types. Compare complete logical trees and assets, including missing plugins and non-ASCII names. [Profiles][profiles] |
| PR-02 | Profile library: create, rename, duplicate, delete, select and bind to device | **Verified subset, C/T.** Native library create/rename/duplicate/delete/select and restart persistence implemented for one Plus device; multi-device bindings remain open. | Perform each operation through native GUI, restart, and verify stable per-profile identities and device associations. [Preferences][preferences] |
| PR-03 | Pages: create/reorder/delete, next/previous/go-to and page indicator | **Verified subset, A/C/T.** Native page CRUD/reorder, next/previous/indicator and swipe navigation implemented. Held controls release before disappearing; stable pinned contexts retain ownership. Original one-based go-to settings are implemented; physical swipe acceptance remains open. | Import both shipped Plus profiles; change every page with GUI, action and swipe; verify boundary behavior and action disappearance/appearance ordering. [Pages release][pages] |
| PR-04 | Nested folders, return-to-parent and configured auto-exit | **Verified subset, A/T.** Shipped folder/back links navigate, with per-profile folder cursor restoration. Configured auto-exit remains missing. | Traverse nested folders from shipped profiles; return to exact parent/page; check per-folder and per-action auto-exit timers. [Auto folder exit][autoexit] |
| PR-05 | Pinned actions/folders with page-specific overrides | **Verified subset, C/T.** Pins span pages/folders, local overrides win, hidden pins have a dedicated editor, and unpin preserves placements. Mapping the Default page to pin storage remains an inference until a nonempty Windows pin export is obtained. | A pinned slot appears across pages, a page override wins locally, and unpin preserves intended assignments. Do not assume an empty Default page means pins are unsupported. [Pinned actions][pinned] |
| PR-06 | Export profiles and action-sharing files; interoperable import/export round trip | **Partial, A/C/T.** Original-format profile ZIP export preserves source fields, assets and duplicate-page references; both shipped Plus round trips pass. Windows app acceptance and action-sharing files remain unverified. | Linux export imports into official Windows app; Windows export reimports into Linux with equivalent pages, folders, actions, settings and assets. [Profiles][profiles], [sharing][sharing] |
| PR-07 | Backup/restore all profiles and migrations without data loss | **Missing, A/C.** Binary contains `ProfilesV2`, `ProfilesV3` and a migration literal; no migration control flow was recovered. | Restore `.StreamDeckProfilesBackup`; test interrupted migration, old backups, rollback and missing-plugin preservation against real fixtures. [Backups][backups] |
| PR-08 | Smart Profiles switch according to focused application | **Missing, C.** No focus monitoring or application bindings. | On supported Linux desktop sessions, focus configured apps, return to default, and verify editing does not cause disruptive switching. Record compositor-specific restrictions honestly. [Smart Profiles][smart] |
| PR-09 | Plugin-bundled profiles and authorized profile switching | **Verified subset, A/C/T.** Unchanged Lights Out auto-imports its Plus profile and runs real game/image logic. SDK switching validates registration, device, declared profile and page; read-only and persisted bundle bindings tested. | Unchanged official Lights Out plugin installs/activates its declared profile and returns correctly. Validate device/page and manifest profile-access rules. [SDK profiles][profiles] |
| PR-10 | Embedded resources, dependency discovery and repair of missing plugins | **Partial, A/C/T.** Unknown action UUIDs are retained/inactive. Soundboard and SDK resource files import/export with original basenames; resource export records plugin dependencies. Automatic missing-plugin installation remains absent. | Import a 7.1+ profile containing media/resources; install a missing plugin and regain function without recreating action settings. [Resources][resources], [7.1 release][release71] |

## Built-in actions and action composition

The shipped profiles contain real action identifiers and settings; their presence is evidence of a compatibility input, not of our implementation. At this baseline, host dispatch resolves installed plugin actions only. Imported built-ins remain inactive.

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| AC-01 | Open application/file/folder, close application, open URL and background request | **Verified subset, A/C/T.** Native Website/Open editors and Linux argv-based execution; actual local background HTTP success/error and executable launch verified. Browser association, Close and Open Application activation/long-press remain open. | Configure through GUI and import fixtures; verify Linux path/app translation, URL handling and explicit background-request behavior. Unmappable Windows paths remain visible and editable. [System actions][system] |
| AC-02 | Hotkey, hotkey switch, modifiers, text entry and optional Enter | **Partial, A/C/T.** Native shortcut editor, Qt-to-evdev translation, held ownership and exact Plus trigger indices implemented. uinput device creation/removal verified, but event readback skipped for missing event-node read access. Compositor shortcuts, arbitrary layouts, Coalesce, hotkey switch and text remain open. | Exercise a named Linux application under supported X11/Wayland sessions; verify Unicode, modifier release, repeated presses, focus and held keys without stuck input. [System actions][system] |
| AC-03 | Multimedia transport, mute and volume | **Verified subset, A/C/T/H/U.** User confirmed physical System Volume/mute. Native Pulse control now adds live percentage/mute feedback, external-change monitoring and configurable tick steps. Owned-sink tests cover writes and display; updated real-device step acceptance and broader transport/player behavior remain open. | Play/pause/seek where supported, mute and change volume against real media players; verify externally changed state feeds back. [System actions][system] |
| AC-04 | Soundboard playback, looping, overlap/restart, fades, volume and output device | **Verified subset, A/C/T.** Native controls, independent streaming mixer and Pulse/PipeWire transport implement four playback modes, volume, fades, routing and soundboard-wide stop. Captured virtual-sink PCM verifies WAV/embedded MP3, volume and fade envelopes. Audible/physical acceptance, Windows timing/visual equivalence and multichannel downmix remain open. | Play supported audio fixtures in each documented mode, route to selected output, stop/retrigger and restart host safely. Verify captured audio and audible output. [Soundboard][soundboard] |
| AC-05 | Multi Actions, delays, switch/random variants and plugin multi-action context | **Verified subset, A/C/T.** Native ordered-step editor, delays, Switch and Random run through an incremental scheduler. Child SDK payloads, desired states, settings/PI ownership, cancellation and restart identities pass real-process tests; unchanged Hello World runs nested. Physical shortcuts and full Windows workflow equivalence remain open. | Import/reorder sequences, check measured timing and stateful branches, then verify plugin events with a fixture that consumes multi-action state. Inventory switch/random behavior before marking those variants passed. [Multi Actions][multi], [delay controls][delays] |
| AC-06 | Dial Stacks | **Verified subset, A/C/T.** Native editing, selected-child lifecycle, click/held-turn/touch routing, fast navigation and persistence work. Unchanged Layouts instances produce verified PNG changes. Physical acceptance and exact artwork/restore behavior remain open. | Load multiple real dial actions; dial push cycles active action and corresponding touch display; settings persist independently. [Dial Stacks][stacks] |
| AC-07 | Action Trigger adapts key actions to dial directions/press | **Verified subset, A/C/T.** Native three-slot editor, key-event routing, timed rotation/touch pulses, normal plugin states, nested sequences and Dial Stack integration implemented. Unchanged Counter and all eleven original records pass process tests; physical gesture acceptance remains open. | Configure three different actions, exercise both rotation directions and press, and verify corresponding plugin key-event semantics. [Action Trigger][trigger] |
| AC-08 | Action Wheel and Key Logic/Key Stack behaviors | **Missing/Unresolved, A/C.** Shipped `keys.logic`/`keys.stack` descriptors exist; semantics are not fully investigated. | Implement documented wheel selection/activation and record actual 7.6 Key Logic/Stack reference behavior before creating differential tests. [Action Wheel][wheel], [7.4 fixes][release74] |
| AC-09 | Device brightness, sleep/screensaver and profile/page navigation actions | **Partial, A/C/T/H.** Profile/page/folder navigation and original brightness key/dial modes, held repeat, cancellation and percentage feedback are implemented and process-tested. Linux repeat uses the original fallback 500 ms/100 ms timing. Original Sleep action and software PNG/JPEG/GIF screensaver, wake consumption, cached-profile restore and lock simulation now pass process tests. Firmware fade-to-zero without a software image and multi-device targeting remain open. | Execute user-configured actions on hardware, verify ranges/state restoration, and ensure navigation targets the correct device. [Preferences][preferences], [screensaver][screensaver] |
| AC-10 | Built-in/tutorial onboarding and meaningful errors for unavailable actions | **Partial, A/C/T.** Native editor explains unavailable imported actions and retains their data; tutorial HTML registers. Broader tutorial screens and every default-profile action remain open. | Import untouched default Plus profile; every action either works or has an actionable visible compatibility error. Run tutorial screens through the supported browser runtime. [Artifact evidence](../../evidence/elgato-7.6/findings.md) |

## Plugin compatibility and rendering contract

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| PL-01 | Node plugins, manifest/runtime version selection and lifecycle | **Verified subset, T.** Unchanged Counter and Layouts register using installed Node 20. Generic entries plus unambiguous portable Windows/macOS script entries are supported, including `.cjs`; installed runtime lookup is deterministic. Original-SDK process fixtures verify both portable fallback forms and documented Node flags. | Test Node 20/24, version mismatch, startup failure, shutdown/restart and plugin supervision; respect manifest versions and platform-specific paths. [Manifest][manifest], [environment][environment] |
| PL-02 | Legacy HTML plugin runtime | **Verified subset, A/C/T.** Native hidden HTML runtime registers unchanged shipped tutorial; generated HTML backend/PI fixture exchanges real SDK messages. Full tutorial navigation and background-media behavior remain open. | Run untouched tutorial HTML with the expected injected connection callback; verify browser networking, local resources, lifecycle and actual responses. [Artifact findings](../../evidence/elgato-7.6/findings.md) |
| PL-03 | Native plugins and OS-dependent modules | **Partial/Unresolved, C/T.** Native Linux ELF plugins launch directly with private data directories and existing supervision. An independent Rust Discord plugin preserves 17 action identities and implements their request routes with mock IPC/auth coverage, notification counts, server statistics and dynamic volume graphics. Live authorization, avatar layouts and long-press notification actions remain open. Unchanged Image Resizer runs in place with its Linux Sharp native dependency. Windows/macOS executable plugins and platform-only dependencies remain unresolved. | Inventory Windows/macOS-dependent plugins, provide a supported runtime/porting path, and test real native binaries or Linux ports against the same contract. Explicit per-plugin limitations remain until resolved. [WebSocket API][ws], [manifest][manifest] |
| PL-04 | Property-inspector browser, registration and bidirectional messaging | **Verified subset, C/T.** Native inspector windows register, route messages/settings, enforce per-action context binding and send appearance/disappearance. Original Counter PI registers; its unchanged control code saves via explicit instrumentation. | Unchanged official inspector registers, receives action/settings metadata, exchanges `sendToPlugin`/`sendToPropertyInspector`, and gets appearance/disappearance events. [PI guide][pi] |
| PL-05 | Device/action lifecycle and physical input events | **Verified subset, D/C/T/H.** Registration, offline device state, connect/disconnect and key/dial events work. SDK stores retain visible actions during same-device recovery; new identity is announced before appearance. Broader physical lifecycle acceptance remains open. | Contract fixtures cover disconnect/reconnect, page/folder transitions, simultaneous instances, title changes and touch coordinates; then repeat physical paths. [WebSocket API][ws] |
| PL-06 | Action/global settings, correlation IDs and persistent isolation | **Verified subset, T.** Sample settings survive restart; global storage and correlated replies exist in code. | Test SDK v1/v2/v3 clients, PI-origin updates, concurrent gets, multi-plugin isolation, malformed settings and crash-safe persistence. [Settings][settings], [SDK 3 changes][sdk3] |
| PL-07 | Title/image/state commands and separate hardware/software targets | **Partial, C/T.** Combined-target per-state SDK updates, including inactive states, user-override priority/reset and titleParametersDidChange packets are implemented and process-tested. Separate hardware/software targets remain explicitly unsupported. | Verify combined and separate targets, every valid state, ownership, title parameters and reset behavior with official SDK fixtures. [WebSocket API][ws] |
| PL-08 | Image formats, animation and resource path behavior | **Partial, C/T.** PNG/JPEG/GIF and SVG file/data-URI rendering work; SVG transparency is checked. Animation and WebP remain missing. | Test file/data-URI icons, SVG and animated assets, scaling/transparency, frame timing and bounded memory with real devices. [Keys][keys], [6.0.1 release][release601] |
| PL-09 | All built-in feedback layouts and custom JSON layouts | **Verified subset, C/T.** All six built-ins and four official custom layouts run unchanged with real preview-pixel checks. Incremental properties, SVG, clipping/layers/gradients work. Exact Windows text/bar raster parity remains unverified. | Run every unchanged Layouts sample action; compare text, pixmap, bars, ranges, z-order, clipping, opacity and incremental updates. [Layouts][layouts] |
| PL-10 | Visual alerts, success feedback and trigger descriptions | **Verified subset, D/A/C/T.** Native-duration key badges, red dial flashes, Trigger miniature feedback, selected Stack forwarding and runtime gesture hints implemented. Generated probes and unchanged Image Resizer warning/success pass. Multi parent routing, pixel-perfect vendor artwork and physical acceptance remain open. | Observe correct temporary overlays and restoration on keys/dials; inspector descriptions update when requested. [WebSocket API][ws], [dials][dials] |
| PL-11 | Embedded plugin resources | **Verified subset, D/A/C/T.** get/setResources, complete action envelopes, correlated replies, opposite-side PI/plugin notifications and portable files implemented. Generated fixtures call unchanged SDK 2.0.4, including nested bindings, duplication and explicit clears; Windows acceptance remains open. | Save/update resources through unchanged plugin/PI, export profile, import elsewhere and reopen the file successfully. [Resources][resources] |
| PL-12 | App monitoring, wake notifications, deep links, openURL and localization | **Missing/Partial, C.** `logMessage` exists; external/system callbacks and plugin localization are absent. | Trigger app launch/exit, suspend/resume and routed URI callbacks; test translated metadata/UI and external browser operations. [Monitoring][monitoring], [deep links][deeplinks], [environment][environment] |
| PL-13 | Plugin crash handling, timeouts, upgrades and removal while actions remain | **Verified subset, C/T.** Launch failures, missing/malformed folders and process crashes stay local to the plugin. Native status/error/log details and explicit Restart preserve action settings and unaffected plugin PIDs. Per-launch and per-socket identities reject stale traffic; a 15-second registration warning leaves the process available for inspection. Automatic restart, disable, upgrade and removal policies remain open. | Crash one plugin without freezing host/other plugins; recover with action settings intact; test repeated failures and upgrade rollback. [Environment][environment], [6.0.1 fixes][release601] |

## Installation, Marketplace and content

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| IN-01 | Install `.streamDeckPlugin` archives as inert validated files | **Verified subset, A/C/T.** Archive path/UUID/size checks, private staging and atomic no-overwrite installation remain. Supported protected manifests authenticate using an explicit pinned local original executable; required files are checked with the recovered BLAKE3 contract before staging and on every plugin load/restart. The downloaded Discord package passes inert install; its original native backend remains unsupported. Optional files follow the original Required policy. | Broaden protected-format/reference-version corpus; native file-picker/drop integration and discovery must preserve validation and runtime limitations. Retain malicious archive and collision tests. [Plugin guide][plugins] |
| IN-02 | Plugin discovery, enable/disable, update, uninstall and dependency handling | **Partial, C/T.** Native installation and a plugin manager expose status, errors and explicit restart; failed and missing plugin paths persist for repair. Enable/disable, update and uninstall remain missing; existing install cannot be overwritten. | Manage plugins from native UI; preserve profile references/settings across removal/reinstall; handle versions and missing dependencies visibly. [Plugin guide][plugins], [backups][backups] |
| IN-03 | Marketplace browse/account/install handoff and authorized entitlements | **Missing/Unresolved, C.** No account or service integration. | Use documented authorized service/browser workflows with the user's account; validate handoff, installation and failure states. An external link alone does not prove install/account parity. [Marketplace workflow][plugins] |
| IN-04 | Icon/background/screensaver packs and file associations | **Missing, C.** Individual raster assets can be read; no pack catalog/import UI or desktop MIME associations. | Import actual official packs; select assets through native libraries; open supported package/profile files from Linux file manager. [Screensaver guide][screensaver], [7.6 backgrounds][release76] |
| IN-05 | Music/SFX content and local playback | **Retired service + Missing playback.** Official guides say Marketplace music/SFX library was removed 15 November 2025. | Preserve and play users' locally owned audio. Do not promise downloads from a retired catalog; replacement providers are separate scope. [Official removal notice][sounds], [soundboard][soundboard] |

## Hardware, device settings and desktop integration

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| HW-01 | Stream Deck+ discovery, serial selection and device identity | **Verified subset, H.** Recorded real `0fd9:0084` and firmware `2.0.3.4`; one active device per host. | Native device picker, correct serial persistence, attach/detach while app is running and another device present. [Plus HID][hidplus] |
| HW-02 | Eight key images and full touch-strip output | **Verified subset, H/U.** Real JPEG writes and user confirmation recorded in README; no new hardware test in this document pass. | Repeat across restarts/profile changes; measure sustained updates and verify orientation, no stale slots and no dropped frames under plugin load. [Plus HID][hidplus] |
| HW-03 | Key transitions, dial push/release, rotation while pressed | **Verified subset, H/T.** Live key/dial inputs observed and decoder tests exist. | Exercise all positions, simultaneous/rapid controls and reconnect; verify exact plugin events and no stuck press state. [Plus HID][hidplus] |
| HW-04 | Touch tap/hold and swipe navigation | **Partial, C/T.** Tap routing and swipe page navigation implemented; full physical touch/hold/swipe acceptance remains open. | Physically tap/hold all four segments and swipe between real pages. Compare local coordinates, direction, hold semantics and visible effects. [Touch guide][touch] |
| HW-05 | Per-action/dial/whole-strip backgrounds | **Partial, C.** Fixed renderer background and imported key images; full touch background layers absent. | Configure and import all three official touch-strip layers; verify composition and retained settings. [Touch guide][touch] |
| HW-06 | Brightness, naming, sleep, lock behavior, screensaver and wake | **Partial, A/C/T/H.** Native per-device brightness preferences and reconnect replay are implemented; unset devices receive no default writes. Raw firmware idle timer getter/setter completed a 0→60→0 second hardware round-trip with independent restoration readback. Software timer/image/GIF sleep, native preferences, wake draining and KDE/logind lock observation are implemented. Full-LCD and all nine restore transfers have HID acceptance evidence. Brightness luminance, real wake/lock acceptance, naming, crop/zoom, standby image and firmware fade path remain open. [Sleep protocol notes](SCREEN_SAVER_PROTOCOL.md). | Set values through GUI, observe physical device; lock/unlock and timer wake restore correct profile without spurious actions. [Preferences][preferences], [screensaver][screensaver] |
| HW-07 | USB reconnect, suspend/resume and startup permission handling | **Verified subset, D/A/C/T/H.** Native offline startup, error UI, serial-specific retry and latest-frame replay implemented. SDK lifecycle tests and controlled real HID handle close/reopen pass. Physical cable removal, re-enumeration and suspend/resume remain unverified. | Unplug/replug same and different serials, suspend/resume, and start without access; recover automatically with actionable permission UI and intact state. [Device protocol][hidgeneral] |
| HW-08 | Multiple devices and cross-device profile actions | **Missing, C.** One connected device per host instance. | Operate two devices concurrently with independent profiles; switch one device from another and test removal while the other remains active. [Multiple devices][devices] |
| HW-09 | Other models, Neo Infobar, + XL, Mobile/virtual devices and Network Dock | **Partial, D/A/C/T.** Model descriptors, documented-family HID transports, device-specific key/dial counts, Neo infobar rendering and page sensors, SDK geometry, model-preserving profiles and an adaptive TypeScript canvas are implemented. Model/process/native tests are recorded in [Device models](DEVICES.md). Original 2017 key/image/brightness support is recovered with firmware-specific codecs; Neo sensor lights track navigation availability. Only Plus has physical acceptance; original-model sleep/rotation, Pedal profile orientation, Studio, Mobile/virtual and Network Dock remain unresolved. | Inventory 7.6 supported surfaces and transport contracts; implement each, then run real hardware/network/mobile acceptance. Simulated layouts alone are insufficient. [7.6 release][release76], [7.1 release][release71], [layouts][layouts] |
| HW-10 | Firmware inspection/update workflow | **Partial, C/H.** Current firmware can be read; updater absent. | Separate update eligibility from write operations; validate correct model/image and recovery using authorized devices. Retain explicit unresolved status until a safe supported update path exists. [Preferences][preferences], [7.1 release][release71] |
| OS-01 | Native package, launcher, autostart and clean uninstall | **Partial, C/T/H/U.** Per-user optimized binary and validated desktop launcher installed, settings migrated with original retained. Opt-in per-user XDG autostart, disable masking and exact launch arguments are tested through GIO. Clean-machine uninstall and actual next-login acceptance remain open. | Install/uninstall on a clean supported Linux system; launcher works outside repository; data migration/retention and login lifecycle are verified. User desktop requirement; [7.6 autostart fix][release76]. |
| OS-02 | Audio routing and per-app/input/output controls | **Partial/Unresolved, A/C/T/H/U.** Native default-output volume/mute and soundboard output routing are implemented. Per-app/input controls and the imported Wave Link integration remain unresolved. | Imported actions control real Linux input/output/app streams with equivalent mute/volume feedback and reconnect behavior. A placeholder volume number is insufficient. [7.1 plugin note][release71], [soundboard][soundboard] |
| OS-03 | OAuth/account-backed and companion-app integrations | **Missing/Unresolved, D/A.** No account UI/credential flow; platform companion apps may be unavailable on Linux. | Keep a named compatibility matrix for real plugins and required services; validate actual login, refresh, actions and logout using supported APIs. Preserve unresolved dependencies instead of claiming universal compatibility. [7.6 plugins][release76], [preferences][preferences] |
| OS-04 | MCP/virtual-deck control introduced before 7.6 | **Missing/Unresolved, D/C.** No MCP endpoint or virtual-deck runtime. | Inspect the official 7.4 contract; authorize a client, enumerate/configure its virtual deck, and exercise real actions with connection/revocation tests. [7.4 release][release74] |
| OS-05 | Long-running reliability, isolation and recovery | **Partial, T.** Bounded sample tests and ordinary persistence work; soak/cross-process ownership evidence absent. | Run representative profiles/plugins through a working day, restart/crash/power-loss simulations and competing host instances; verify bounded memory, no lost config and no duplicated actions. Project acceptance requirement. |

## Artifact facts and baseline source identity

The official installer was inspected statically. It was not executed to prove Windows behavior. [Findings](../../evidence/elgato-7.6/findings.md) and [machine observations](../../evidence/elgato-7.6/shipped-observations.json) distinguish REA/Ghidra evidence from Python ZIP/JSON observations. The focused function analysis did **not** return decompilation; it must not be cited as recovered control flow.

| Evidence | Pinned identity and result |
| --- | --- |
| Windows executable | SHA-256 `2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`; plugin registration/browser/native runtime literals and profile migration identifiers observed. |
| Plus default profile A | `raw/fls1.KgkvZ_osjTGmFQpIDeeaNhGPU`, SHA-256 `05ed051d2c0a84d2b989748ce02dc7e2b0db4dd0de9dd227c5ea4b62b37468ee`; current page has six keys/four dials, eight embedded pages, empty Default page. |
| Plus default profile B | `raw/flsysF8Fq6CoTq162E1zc6fyeIlKcw`, SHA-256 `0aed9cbf63dc739d71a21a981b65f052d0a2d163124ef3c28044d0c5b61a8ca3`; same page/control structure, different action settings. |
| Tutorial HTML plugin | `raw/flsCs3U5bpGlHPaoJZvml3X4InTgss`, SHA-256 `ee19cf965f58597e51b33abef1207ba6fc672fe31b851f05545d7cdb15275866`; `app.html`, `inspector.html`, plugin version `1.0.4.3`. |
| Official sample corpus | [Elgato repository](https://github.com/elgatosf/streamdeck-plugin-samples/tree/8cc82bad8716c000bacc131285ffcf6805dafff9), commit `8cc82bad8716c000bacc131285ffcf6805dafff9`; fixtures only, no Linux implementation copied. |

Tests already run during the preceding implementation pass: [sample interoperability](../tests/sample_interop.py) ran unchanged Counter/Layout plugins in simulation, exercised state changes and restart persistence; profile/package tests validated official archives and hostile ZIP cases. These checks do not cover the missing desktop rows. Hardware/user findings above are recorded prior-session evidence from [README](../README.md), not freshly repeated by this documentation task.

No repository commit existed for the local application snapshot. Source inspection anchors:

| File | SHA-256 at baseline inspection |
| --- | --- |
| `src/main.rs` | `e49c6832cb1496e1e4a9a02a6fb92c46be47b49d1f9ab9af4f1bc1c39861214a` |
| `src/host.rs` | `18fc11fb7b3d78433e26281a9c18f3031c130a20832d912c958566a6020ce5ca` |
| `src/ui.html` | `ff33ac0f7639202cb31e9113f58c2921bcfdf1f3d4726ea5f2db1652154a3e73` |
| `src/profile.rs` | `b3d8db484190740ff71dc9b8733af470e6f892822fb528dcc53f7f9370f9885e` |
| `src/package.rs` | `c2f4334a0c16fe77798c1f20b64fe037dee71e94053ab20b69643972a58167a3` |
| `src/hardware.rs` | `65783f0164f84067e06e65005dd05d76d563a565fee2c3f0ab4d147b22e2c3d9` |
| `src/render.rs` | `c009d32e6f4bada2b8bd21ef394bc2a1dbca720de6f6c9723b991761dc13ae07` |

## Completion gate

Full parity remains open until required rows have evidence at their actual usage boundary. Implementation order can prioritize native editing, reusable profiles and the attached Plus without changing that definition.

For every row promoted to verified, record the app build/source revision, reference version, input fixture hash, expected/observed result and evidence level. For every service or native plugin that still cannot work, retain an explicit compatibility entry with the exact cause and remaining work. Do not convert “not tested,” “unsupported,” “logged only,” or “requires a different program” into a passing status.

Before a final parity claim, replay representative user workflows from a clean Linux install: create/edit a profile in the native app; import and export existing profiles; navigate pages/folders/stacks; execute built-ins and unchanged plugin actions; configure through real property inspectors; install/update/remove plugins; reconnect/suspend/resume; and restore a backup. Repeat device-specific behavior on actual devices. Reconcile the complete official 7.6 feature inventory, including features still marked unresolved here.

## Primary references

All external links below are Elgato-owned documentation. Help Center pages that failed HTML retrieval were read through the official `/api/v2/help_center/en-us/articles/<id>.json` endpoint; links retain their human-readable article URLs. Research date: 2026-10-11. Later SDK documentation may include features beyond 7.6; check each feature's version gate against the pinned desktop target.

[release76]: https://help.elgato.com/hc/en-us/articles/51156609278737-Elgato-Stream-Deck-7-6-Release-Notes
[release74]: https://help.elgato.com/hc/en-us/articles/45347482546193-Elgato-Stream-Deck-7-4-Release-Notes
[release71]: https://help.elgato.com/hc/en-us/articles/41533810232721-Elgato-Stream-Deck-7-1-Release-Notes
[release601]: https://help.elgato.com/hc/en-us/articles/10866430132109-Elgato-Stream-Deck-6-0-1-Release-Notes
[plugins]: https://help.elgato.com/hc/en-us/articles/33589587352337-Elgato-Stream-Deck-Download-and-use-Plugins
[preferences]: https://help.elgato.com/hc/en-us/articles/360028236311-Elgato-Stream-Deck-Preferences
[profiles]: https://docs.elgato.com/streamdeck/sdk/guides/profiles/
[pages]: https://help.elgato.com/hc/en-us/articles/5162770312717-Elgato-Stream-Deck-5-1-Release-Notes
[autoexit]: https://help.elgato.com/hc/en-us/articles/22589649057933-Elgato-Stream-Deck-Auto-Folder-Exit
[pinned]: https://help.elgato.com/hc/en-us/articles/26638431612429-Elgato-Stream-Deck-Pinned-Actions-and-Folders
[sharing]: https://help.elgato.com/hc/en-us/articles/29655138607505-Elgato-Stream-Deck-Action-Sharing
[backups]: https://help.elgato.com/hc/en-us/articles/360048424432-Elgato-Stream-Deck-How-to-Back-Up-and-Restore-Profiles
[smart]: https://help.elgato.com/hc/en-us/articles/360053419071-Elgato-Stream-Deck-Smart-Profiles
[system]: https://help.elgato.com/hc/en-us/articles/360028234471-Elgato-Stream-Deck-System-Actions-Hotkey-Open-Website-Multimedia
[soundboard]: https://help.elgato.com/hc/en-us/articles/360059017631-Elgato-Stream-Deck-Soundboard
[multi]: https://help.elgato.com/hc/en-us/articles/360027960912-Elgato-Stream-Deck-Multi-Actions
[delays]: https://help.elgato.com/hc/en-us/articles/7092527123725-Elgato-Stream-Deck-Customizable-Multi-Action-Delays
[stacks]: https://help.elgato.com/hc/en-us/articles/10843581380109-Elgato-Stream-Deck-Dial-Stacks
[trigger]: https://help.elgato.com/hc/en-us/articles/29655069662609-Elgato-Stream-Deck-Action-Trigger
[wheel]: https://help.elgato.com/hc/en-us/articles/28742375481997-Elgato-Stream-Deck-Action-Wheel
[screensaver]: https://help.elgato.com/hc/en-us/articles/26543873678477-Elgato-Stream-Deck-How-to-set-an-Image-or-GIF-as-a-Screensaver
[sounds]: https://help.elgato.com/hc/en-us/articles/4402425921165-Elgato-Stream-Deck-Download-and-use-Sound-Effects-Sounds
[devices]: https://help.elgato.com/hc/en-us/articles/4424235832717-Elgato-Stream-Deck-Use-multiple-Stream-Deck-devices-at-the-same-time
[touch]: https://help.elgato.com/hc/en-us/articles/10567698991629-Elgato-Stream-Deck-Touch-Strip
[manifest]: https://docs.elgato.com/streamdeck/sdk/references/manifest/
[environment]: https://docs.elgato.com/streamdeck/sdk/introduction/plugin-environment/
[ws]: https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/
[pi]: https://docs.elgato.com/streamdeck/sdk/guides/ui/
[keys]: https://docs.elgato.com/streamdeck/sdk/guides/keys/
[dials]: https://docs.elgato.com/streamdeck/sdk/guides/dials/
[layouts]: https://docs.elgato.com/streamdeck/sdk/references/layouts/
[settings]: https://docs.elgato.com/streamdeck/sdk/guides/settings/
[sdk3]: https://docs.elgato.com/streamdeck/sdk/releases/upgrading/v3/
[resources]: https://docs.elgato.com/streamdeck/sdk/guides/resources/
[monitoring]: https://docs.elgato.com/streamdeck/sdk/guides/app-monitoring/
[deeplinks]: https://docs.elgato.com/streamdeck/sdk/guides/deep-linking/
[hidplus]: https://docs.elgato.com/streamdeck/hid/stream-deck-plus/
[hidgeneral]: https://docs.elgato.com/streamdeck/hid/general/

## Native desktop evidence update

On 2026-10-11, the native Rust application was built with eframe0.36.2, installed
per-user, and launched on the attached Stream Deck+. The editor calls
`host::Controller` directly; its display/control path does not require HTTP or
a browser. The legacy web interface remains a headless diagnostic mode.

- **T:** 27 tests including direct embedded controller lifecycle/session-lock release,
  draft preservation/validation, ownership checks and existing protocol/profile tests.
- **T:** real unchanged Counter/Layouts interoperability still passes after refactor.
- **C/T:** optimized executable, validated desktop entry, and app-owned screenshot
  with clean plugin shutdown; no browser window is used for native capture.
- **H/U:** user confirmed native settings edits and physical key response both work.
- **Artifacts:** `../../evidence/native/desktop.png`, `capture.log`, `migration.json`,
  `verification.json`; original workspace session retained.

These results update UI-01/UI-02 subsets. Property inspectors, HTML/native plugin
runtimes, layout coverage, page/folder management and the rest of this ledger
remain open. Full-goal completion has not been established.

## HTML and feedback evidence update

The current runtime implements native HTML backends and separate property-inspector
windows. `tests/webview_interop.py` distinguishes a generated conformance fixture,
an instrumented real Counter control, unchanged direct Counter inspector
registration, and unchanged tutorial registration. These are different evidence
levels; none establishes all third-party plugins or every tutorial screen.

`tests/layout_interop.py` executes six built-in and four custom layouts through
the hash-pinned official plugin and inspects actual PNGs. The missing official
C1 icon is explicitly diagnosed rather than replaced. `src/layout.rs`,
`src/render.rs` and `src/host/display.rs` implement the independent renderer.
Original-format profile/navigation, native platform binaries, built-in actions
and all other open rows remain part of the goal.

### Verification update, 2026-10-12

50 tests and strict Clippy pass. Optimized-process suites cover unchanged Node
samples, all official layout sample modes, native HTML/PI contracts, and native
window signal shutdown. A generated HTML backend opens a native popup whose
child JavaScript executes and communicates with its opener. The unchanged
Counter inspector also registers through its original manifest.

Native mode now exposes its plugin WebSocket only; the browser editor/control
HTTP routes are confined to the explicit diagnostic `serve` mode. This is a
verified transport distinction, not a claim of complete desktop parity.

The following profile pass adds ownership-aware retirement: late messages from
the owning plugin are ignored for removed contexts, while hidden-page settings
still persist. Unknown and cross-plugin contexts remain rejected.


### Profile library evidence update, 2026-10-12

The native editor now manages profiles, pages, folders and pinned actions.
`tests/profile_interop.py` drives real plugin lifecycle events, hidden-page settings,
local pin overrides, folder return, export/reimport and restart persistence. It
imports a hash-pinned Windows Plus archive and compares original manifest fields
and asset bytes. Exporter unit tests cover both shipped Plus archives, duplicates,
deletions and root-only profiles. User confirmation of the new native management
and physical swipe workflows is still pending.

`tests/bundled_profiles_interop.py` runs the unchanged Lights Out HTML plugin and
checks actual SVG-backed PNG pixels after a game move. A generated SDK caller
checks declared-profile access, lazy import, read-only edits, return/cursor behavior
and invalid requests without unwanted imports. Neither suite opens USB.

Default-page pin storage is inferred from the available artifacts and official
pin behavior. Windows acceptance of exported archives is not established. Smart
profiles, folder auto-exit, full backups and action-sharing files remain open.

This profile checkpoint passes 69 unit/fixture tests, strict Clippy and all six
optimized-process suites. App-owned native capture uses a copy of the saved
user session. The installed build opens the real Plus and connects Counter and
Layouts; its native HTTP editor routes return 404. Evidence: `../../evidence/profiles/`.


### System action evidence update, 2026-10-12

`../../evidence/system-action-contracts.json` records original settings and
source hashes. `../../evidence/dial-system-contracts.json` adds bounded native
assembly/RTTI/import evidence: dial Hotkeys indices 0 CCW, 1 CW, 2 press/tap;
media stop3/fast-forward13/rewind14, and composite18 volume controls. This is
static shipped-artifact evidence, not Windows runtime or recovered Ghidra source.

`tests/system_interop.py` runs actual localhost GET requests and an isolated
marker executable through the host, checks HTTP failure reporting, responsiveness,
release ownership and null-settings preservation. It emits no input and opens no
USB device or browser. Keyboard ownership/translation/frame tests run without
injection. The separately gated uinput test creates the exact virtual keyboard,
but skips before emitting any event when its evdev node cannot be read/grabbed.
The native editor's new controls have preservation tests and app-owned captures.
These results do not establish real shortcut/media acceptance or full parity.

This checkpoint passes 94 non-injection tests plus the gated test's default
no-op path, strict Clippy, and seven optimized-process suites. The explicitly
enabled uinput probe still skipped event emission because `/dev/input/event31`
was not readable; it verified virtual-device creation/removal only. Logs and
native captures are in `../../evidence/system-actions/`.


### Soundboard evidence update, 2026-10-12

`../../evidence/soundboard-contracts.json` records 15 exact shipped media links,
resource registry/blob hashes, serialized output device identity and bounded
Stop Audio dispatch. The host resolves archive media without changing foreign
source paths. Export tests preserve unchanged media byte-for-byte and embed new
files using the observed FormatVersion1 resource wrapper. Windows import
acceptance is still unverified.

`src/soundboard.rs` owns streaming decode, resampling, voice ownership and fades.
`src/audio_output.rs` owns a nonblocking Pulse connection on a worker thread.
Explicit device routing is checked against the actual connected output before
samples are sent. Generated PCM tests and `tests/soundboard_interop.py` distinguish
pure mixing from actual server-output capture; the latter uses an owned null sink
and preserves the desktop default. A failed first routing test exposed implicit
server fallback; its log is retained alongside the correction and passing rerun.
No claim of exact Windows fade curves or audible hardware acceptance follows
from these tests. Other feature gaps in this ledger remain open.

This soundboard checkpoint passes 112 non-injection tests plus the default
gated-keyboard no-op path, strict Clippy and eight optimized-process suites.
Captured amplitude halves at 50% volume; both fade envelopes are measured.
Native capture, source/binary hashes and output-routing evidence are retained
in `../../evidence/soundboard/`. The installed build reconnects the real Plus
and original Counter/Layouts plugins; soundboard hardware acceptance is pending.


### Multi Action evidence update, 2026-10-12

`../../evidence/multi-action-contracts.json` records the shipped routine fixture,
its hashes and bounded native dispatch/serializer evidence. Children use
`Actions[group].Actions`; `OverrideState` is distinct from current `State`.
The two timing components are `MultiActionDelays.Delay1` and `Delay2`, each
1–100 ms. Native branches establish repeated-press cancellation, success-only
switch advancement and random selection with possible repeats. Exact page-hide
callers and cancellation-time child cleanup remain partly unresolved.

`tests/multi_interop.py` measures generated real-plugin events and runs the
unchanged official Hello World sample. It checks child PI registration, settings,
configured/manifest timing, desired-state envelopes without canvas coordinates,
cancellation, switch/random behavior, native loopback requests, export and restart.
The original Windows macro imports/exports without executing its system shortcuts.
Pure host tests cover cancellation of already-dequeued events and nested audio
embedding. Native editor drafts retain unknown fields and survive failed saves.
Keyboard initialization is awaited before starting timed sequences; no physical
shortcut timing is claimed by the non-injection test suite.

This Multi Action checkpoint passes 129 non-injection tests plus the gated
keyboard no-op path, strict Clippy and nine optimized-process suites. The native
editor capture, timing/protocol tests, hashes and unchanged Hello World build
evidence are retained in `../../evidence/multi-actions/`. The installed app
connects the attached Plus and all three sample plugins.


### Dial Stack evidence update, 2026-10-12

`../../evidence/dial-stack-contracts.json` records four original stacks, archive/
member hashes, selected-only SDK lifecycle, pending-selection routing and the
fast-navigation deadline. The stack consumes dial pushes; touch and ordinary
turns reach the selected Encoder child. Fast held turns select a pending entry,
and release commits it; the original first clockwise turn advances two positions
because push already queues the next item. The selected child remains visible
until commit. Native fast-navigation restore/editor paths remain qualified.

`tests/stack_interop.py` verifies active/inactive contexts, ordinary Encoder
payloads and coordinates, cached late replies, PI activation, persisted selection,
flat child export and unknown fields. Original Wave Link records, including an
out-of-range placement for the attached Plus geometry, survive export. Wave Link
itself remains unavailable. Actual unchanged Layouts feedback images and icons
are checked; independently drawn marker changes stay inside [160,8,26,26].
The common child store also migrates earlier Multi Action sessions without
changing their contexts or SDK meaning. Native rendering is captured from an
isolated session; no physical stack gesture is claimed by simulation tests.

This stack checkpoint passes 143 non-injection tests plus the gated-keyboard
no-op path and strict Clippy. Ten integration suites passed; Stack, Layouts,
samples and Multi Actions were rerun after the selected-child icon fix. Native
capture and hashes are in `../../evidence/dial-stacks/`. The prior user session
is backed up; its existing settings match the running session, which also has
a new Media control. Physical stack acceptance remains pending.


### System Volume feedback update, 2026-10-12

The user confirmed turning System Volume on dial 2 changes volume and pressing
it toggles mute, then requested a visible percentage and configurable amount.
`src/volume.rs` now reads the default output through native PulseAudio/PipeWire,
monitors changes and applies configured percentage-point steps while retaining
channel balance. The native editor and touch-strip preview show the actual
percentage and mute state. `linuxVolumeStep` is a Linux extension, default 5.

`tests/volume_interop.py` writes only an owned virtual sink. It verifies fractional
steps, balance, mute, external changes, rendered pixels, missing-output errors,
recovery and restart persistence. A separate read-only query of the actual default
output matches the daemon's value. The prior physical confirmation applies to the
previous media-key control path; this new direct-audio implementation's physical
step behavior remains a separate acceptance check. Action Trigger investigation
was retained in `../../evidence/action-trigger-contracts.json` while the requested
volume improvements took priority.

This volume checkpoint passes 150 non-injection tests plus the gated-keyboard
no-op path, strict Clippy and eleven integration suites. Native capture shows
both percentage and step control. The live read-only percentage exactly matches
the audio server, and the installed build connects to the actual Plus and default
AirPods output. Logs, virtual-sink metrics and source/binary hashes are retained
in `../../evidence/system-volume/`; the prior user session is backed up.


### Action Trigger evidence update, 2026-10-12

`../../evidence/action-trigger-contracts.json` and its follow-up retain the eleven
original adaptor records, archive/member hashes and bounded native dispatch slices.
The original format is a flat three-position `Actions` array: CCW, press, CW.
Blank records use `{"Actions":[]}`. Children are ordinary Keypad contexts with
the parent dial coordinates and no `userDesiredState`. Multi Action opt-out does
not exclude these children. Rotation queues use 60 ms holds, 15 ms gaps and a
three-item pending cap; direct physical pushes bypass that queue.

`src/action_trigger.rs` implements the independent parser/hit-test/repeater;
`src/host/trigger.rs` handles actual event ownership and composite rendering.
Cancellation releases only dispatched holds, and overlapping physical/touch holds
share ownership. These are explicit Linux cleanup choices; native extra-release
edge cases are not copied. Half-open hit-rectangle edges remain a Linux convention.
Automatic state advancement follows the [manifest contract][manifest] and recovered
native keyUp path, honoring `DisableAutomaticStates` for ordinary key actions.

Native editors support Dial Stack → Action Trigger → Multi Action navigation,
with per-context drafts and Back. Recursive child hydration/export preserves
unknown fields, resources, positional blanks and identities. Go-to-page dispatch
uses the recovered one-based `PageIndex` and current profile page order.

`tests/trigger_interop.py` runs a generated protocol probe and unchanged official
Counter, measuring pulse timing, cancellation, contexts, nested sequences,
selected-stack lifecycle, persistence and all eleven original adaptor round trips.
Pure tests cover prefetched-event cancellation, edits during queue gaps and shared
hold ownership. Windows execution, exact reference visuals and physical Action
Trigger acceptance are still unverified; broader ledger gaps remain open.

This checkpoint passes 183 non-injection tests plus the explicitly gated keyboard
test's default no-op path, strict Clippy on Rust 1.99, and twelve process suites.
Native app-owned capture verifies the always-dark Trigger editor. The installed
build retains the user's saved controls; hardware startup and sample registration
are checked separately from physical gesture acceptance. Evidence is retained in
`../../evidence/action-triggers/`. Public tests now include MIT-licensed official
layout/profile fixtures; proprietary installers and private sessions stay local.


### Plugin resource evidence update, 2026-10-12

`../../evidence/plugin-resource-contracts.json` records official API/SDK sources,
package hashes and bounded native resource setter/getter/linker paths. The setter
replaces the complete map, removes empty path values and requires remaining paths
to be absolute; registration does not require the file to exist. Getter replies
preserve request IDs. Plugin setters notify the bound inspector; inspector setters
notify the owning plugin. Neither receives its own setter echo. The native linker
reuses an ID for the same path; exact Windows normalization remains untraced.

`src/host/resources.rs` connects these contracts to settings-independent persisted
bindings. The filesystem helper retains imported basenames and uses bounded,
atomic cache writes. Export preserves unchanged original IDs/blobs, shares IDs
for identical Linux path strings, and keeps distinct same-named files separate.
File size and aggregate export limits are checked before accumulating oversized
asset maps. Duplication regenerates contexts while retaining nested overrides and
explicit clears; discarded raw resource IDs cannot resurrect a cleared binding.

`tests/resources_interop.py` uses generated plugins with unchanged official SDK
2.0.4 and native WebKit inspectors. It checks owner isolation, response correlation,
notification direction, replacement/clearing, nested Trigger/Multi bindings,
duplication, export/import after deleting originals and restart. No complete
resource sample was found in the pinned/current official sample repository.
This establishes generated-SDK conformance, not an unchanged end-user resource
plugin or acceptance in the Windows app. Evidence: `../../evidence/plugin-resources/`.

This resource checkpoint passes 198 non-injection tests plus the gated keyboard
test's default no-op path, Rust 1.99 strict Clippy and thirteen process suites.
The installed binary reconnects to the attached Plus and all three sample plugins;
its prior session is backed up and saved control settings are preserved. The first
resource test runs exposed nested controller addressing and duplicate aliases in
the test's snapshot traversal; their logs are retained with the passing final run.


### Plugin feedback evidence update, 2026-10-12

`../../evidence/feedback-command-contracts.json` records official SDK types and
bounded original native spans. Key effects use a transparent still-overlay layer
for 3000 ms, replacing/restarting an earlier effect while base images/titles remain
composited underneath. Action Trigger renders the affected child miniature.
Encoder errors use a separate full-cell red popup, opacity .75 through 165 ms
then linearly fading to zero by 330 ms, with a 20 ms timeline update interval.
Selected Dial Stack children forward to the parent cell; inactive ones do not.
Multi Action child-to-physical-parent forwarding remains unproven.

The independent badge renderer and timed host composition restore the latest
underlying image/state. Transient indicators are not exported or restored across
app restarts. Shutdown clears effects and drains the last normal image before
stopping the USB writer. The native window receives redraw notifications; the
hardware worker skips identical images during animation. Gesture descriptions
use sparse replacement, explicit empty-text hiding and empty-map manifest reset.

`tests/feedback_interop.py` measures actual generated-plugin PNGs and timers,
including ownership, hidden controls, updates during effects and nested scopes.
Its optional unchanged Image Resizer fixture runs both invalid-dimension warnings
and a real temporary-image resize followed by success feedback. Vendor source,
lockfiles and manifest hashes remain unchanged; Linux Sharp is loaded from the
sample's existing dependency tree. This is an in-place sample test, not proof
that Windows-only plugin dependencies work after a package is copied to Linux.

This checkpoint passes 209 non-injection tests plus the gated keyboard test's
no-op path, strict Rust 1.99 Clippy and fourteen process suites. App-owned native
capture shows the effects and gesture hints. Evidence, timing samples and source
hashes are retained in `../../evidence/plugin-feedback/`. Physical effect acceptance
and broader parity still require the other ledger entries to be resolved.


### Offline device and reconnect evidence update, 2026-10-12

This updates HW-07, PL-05 and held-control aspects of HW-03.
`../../evidence/device-lifecycle-contracts.json` records official disconnected-device
visibility and SDK 2.0.4 behavior. Registration can include an offline device;
connection events change its online flag without deleting visible action instances.
A previously unknown device must reach the SDK device store before its action events.
Exact native event ordering and serial/context permanence remain unproven.

`src/device_session.rs` owns reconnecting transport and nine latest display frames.
It pins the selected/first successful serial, retries after failures, prioritizes
input between writes, skips unchanged images and replays the latest state after
opening a fresh handle. Offline retries do not stop plugins or block the editor.
Physical holds are released when transport is lost; completed clicks and their
asynchronous Multi/Trigger work keep running. Changing logical device identity
ends the old action lifecycle before connect/appear for the new identity.
These context-stability and cleanup details are explicit Linux host policies.

`tests/device_interop.py` uses the unchanged official SDK to verify stores, event
ordering, stable plugin PID, resources/settings, held release and timed-work
continuity. Two actual native startups use an explicitly absent serial while the
real deck remains untouched. Fake transport tests exercise failed reads/writes,
retry, frame replay and shutdown. A separately gated probe exclusively opens the
real Plus, injects one controlled read failure, closes/reopens its own HID handle
and reports two opens/two drops/zero image writes. It proves handle recovery,
not physical cable removal or USB re-enumeration. App-owned capture confirms the
editable offline native window. Evidence: `../../evidence/device-lifecycle/`.

This device checkpoint passes 225 non-gated tests plus the two gated tests' default
no-op paths, strict Rust 1.99 Clippy and fifteen process suites. The real handle
probe also ran explicitly and passed after review fixed feature-report-only serial
matching. Exact descriptor matches avoid unrelated probes; devices lacking a serial
descriptor can be matched by a read-only serial feature report. An identity change
closes inspectors carrying old device initialization. The prior live session is
backed up; the installed build preserves its four controls and reconnects the
actual Plus with the three original sample plugins.


### Native action library and plugin recovery, 2026-10-12

The user prioritized UI and plugin support over remaining screensaver work.
The native Actions sidebar searches manifest metadata, groups categories, filters
key/dial compatibility and honors `VisibleInActionsList`. Library-to-canvas drops
use the same checked host assignment command. Ctrl+F focuses search; Escape
clears it. Plugin/profile management has its own window.

`tests/plugin_runtime_interop.py` covers isolated startup failures and crashes,
explicit repair/restart, stale registrations, preserved action settings, unchanged
other-plugin processes and owned-child cleanup. Unit tests exercise socket
generation isolation, registration warnings and bounded runtime probes. The
Counter/Layouts, native inspector, resource and window-lifecycle suites also pass.
Native captures use a separate simulated session, including a failed executable
plugin beside the connected official Counter sample. These checks do not establish
physical drag gestures or Windows executable compatibility.

The downloaded Discord Marketplace package still fails before installation.
Authenticated manifest decryption now works, but the package file-hash contract
remains unresolved. See [plugin compatibility notes](PLUGIN_COMPATIBILITY.md).
No Discord command, login or voice operation was performed.


### Native plugins and Discord replacement, 2026-10-12

`src/plugin_entry.rs` resolves generic scripts and unambiguous portable platform
entries, supports `.cjs`, checks Linux ELF architecture/executable permissions,
and finds Node versions in deterministic order. `tests/plugin_entry_interop.py`
uses the unchanged official SDK. `tests/native_discord_interop.py` installs the new
Rust Discord package, verifies the actual Linux child process and 17 actions,
opens its native inspector, restarts it, retains settings and checks that seeded
private credentials are absent from host state and exported profiles.

The independent Discord crate passes 19 tests, including real loopback WebSocket
and mocked Unix IPC/OAuth. Main host checks pass 317 tests with 10 gated tests
ignored; the actual original protected manifest test also passes when explicitly
provided the local reference and package. Strict Clippy passes for both crates.
These are software/process tests, not evidence of Discord account authorization
or physical control effects. Screensaver completion remains deferred.

### Embedded TypeScript editor and Discord follow-up, 2026-10-12

The default desktop shell now embeds the React/TypeScript production bundle using
Wry/WebKit. Rust still owns profiles, settings, devices, plugins and lifetime. The
editor has a persistent profile sidebar, searchable action library, compact Plus
canvas, independently scrolling inspector and pinned Save/Revert controls. No
editor HTTP service or development server is required. Plugin inspectors remain
in separate windows. `desktop --legacy-editor` retains the earlier UI temporarily.

Frontend checks cover draft preservation, stale-context rejection, unknown settings,
structured child actions, startup and form errors. CUA checks confirm draft edits
survive selection changes, explicit Save works, and dial-volume step controls are
visible. Native WebKit evidence confirms bridge dispatch and real window
destruction/recreation with unsaved drafts retained. The unchanged official SDK
plugin keeps its PID and heartbeat across close/reopen, including without a session
bus. These are software checks; physical acceptance of the new editor remains open.

Core tests pass 322 cases, with ten gated tests ignored. Frontend tests pass 15
cases, and the native Discord crate passes 36. Strict Rust Clippy passes for both
crates. The real-host Discord process fixture verifies rendered volume PNGs, live
updates, notifications, guild counts, stored-session resume without new consent,
and release of held mute controls during host shutdown. All Discord service
responses in these tests are local mocks. Avatar layouts, live authorization and
actual Discord control behavior remain unverified. GitHub CI remains manual-only.

### Wave Link functionality, 2026-10-12

This is an additional compatibility target requested by the user, not a reason to
remove any desktop requirements above. The native routing foundation is described
in [Audio mixer](AUDIO_MIXER.md).

| ID | Required behavior | Current evidence and remaining work |
| --- | --- | --- |
| WL-01 | Independent application/input groups, monitor/stream mixes and physical output assignment | **Verified subset, C/T.** Original Rust Pulse-compatible graph, five mixes/twelve groups, exact app/source/output assignments, separate route/master gain and mute, persisted configuration and native TypeScript matrix. Generated virtual PCM verifies separation, levels, mute and routing primitives. Real microphones/speakers and audible acceptance remain unverified. |
| WL-02 | Mixer key/dial actions and imported official plugin compatibility | **Partial, C/T.** Original Deckforge actions support route/master targets, step size, mute, key set/change operations, Multi Actions and Dial Stacks. Host-process tests verify feedback and restart. Shipped profiles prove six official `com.elgato.wave-link` identities but all settings are empty. Current plugin schema/protocol compatibility remains unresolved; unknown imports remain preserved. |
| WL-03 | Live metering, immediate faders, fades, linked mixes and output controls | **Partial/Missing.** Key/dial actions change controls live; editor changes currently require Apply. Feedback reflects worker-accepted settings, not measured signal peaks. Audio meters, immediate editor faders, fades, linked controls and per-output gain still need implementation and measured verification. |
| WL-04 | Effects, effect switching and Wave hardware/DSP controls | **Missing.** No VST/AU effect hosting or proprietary Wave-device DSP implementation is present. These require separate contracts, supported Linux mappings and hardware evidence. |
| WL-05 | Background operation, disable, shutdown and recovery | **Verified subset, C/T.** Exact owned-node journals and kernel ownership leases preserve parked apps/recorders across normal Quit/restart. Disabled fresh startup leaves audio untouched; explicit Enable recovers matching nodes. Tests verify original-output restoration, respect manual routing changes, silent attached-reader retention and unchanged system defaults. Abrupt crashes and full audio-daemon restart remain unverified. |

`tests/mixer_host_interop.py --run-virtual` uses the actual application, simulated
key/dial input and silent owned nodes. `tests/mixer_virtual.py --run-virtual`
measures generated PCM and recovery with temporary streams. Neither captures the
user's microphone, plays through physical speakers or moves a real application.
Ownership evidence is recorded under `evidence/wave-link/`; local inventories are
excluded from the public repository. Core tests pass 343 cases with 11 gated tests
ignored; frontend tests pass 29 cases. Strict core Clippy passes.

### Action transfers and protected package integrity, 2026-10-12

`tests/action_transfer_interop.py` drives the actual host with the unchanged
official Counter sample and an inert local lifecycle probe. It verifies copy
independence, cut/paste between pages, occupied-slot swapping, stale-target
rejection, held-input release before disappearance, retired-context replies,
nested Multi copies, independent folder trees, and source-image portability.
Two original SVG state assets survive cross-profile transfer, deletion of the
source profile, export and restart. Unit coverage also preserves animation and
shared resource bytes, explicit resource clears, pin ownership and persistence
failure behavior. Runtime contexts renew on moves to refresh plugin coordinates.

Native WebKit/React tests click Copy/Paste and verify separate Counter contexts
and saved settings. CUA checks cover the Edit/right-click menus and Shift+F10.
Normal system clipboard integration, action-sharing files and general undo remain
open. Cross-profile folder moves preserve unused source pages conservatively.

Static analysis identifies the original required-file checker as BLAKE3 with a
32-byte result compared to Base64 Hash. All 179 listed Discord package members
match, of which 54 are required. The production reader authenticates the manifest
and verifies required members before staging and on installed-folder reload.
Process tests cover missing/tampered required data, optional-member changes,
symlink/FIFO rejection, unchanged protected-manifest bytes and a visible native
runtime failure with no process launched. No vendor backend, Wine, account login
or real Discord command is used. See [protected packages](PROTECTED_PACKAGES.md)
for the supported reference and stricter parsing boundaries.

Core checks pass 354 tests with 11 opt-in tests ignored; frontend checks pass 39.
Existing official whole-profile, resource SDK/native inspector and portable Node
entry suites pass after the provenance and file-reader changes. Physical editing
acceptance and complete Windows application parity remain unverified.

### Appearance editing, 2026-10-12

The native editor now edits custom icons and title styling per state. Previews
run through the Rust renderer without saving or sending frames to the device.
A native React/Counter fixture verifies that an appearance draft remains local,
that Save retains a count changed by the running plugin during editing, and that
plugin title ownership remains intact. Extensionless manifest icons survive style
changes and resets. Titles support multiple lines.

`tests/appearance_interop.py` checks actual rendered pixels, sparse style patches,
SDK title-parameter packets, inactive-state title/image updates, user priority,
reset, original SVG export/reimport and restart. Unit tests cover two-state GIF/SVG
byte preservation and a high-entropy photo whose decoded cache is bounded for safe
session reload. Imported profile override provenance remains partly inferred from
standard raw Image/Title fields; new SDK runtime caches are kept out of user-profile
exports. See [Action appearance](APPEARANCE.md) for exact limits.

Core tests pass 374 cases with 11 opt-in tests ignored; frontend tests pass 49.
Font matching/loading runs outside the shared render-cache lock. Windows font
metrics, complex shaping, per-character fallback and animated key playback are
not established. The user's next priority is other devices, then plugins, then
Wave Link meters. Long-running reliability checks are delegated to the user.
