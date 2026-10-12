# Stream Deck desktop feature parity

Target: a native Linux desktop application written in Rust, with the behavior of Elgato Stream Deck **7.6.0.23012**. Existing Elgato profiles and plugins remain first-class compatibility requirements. A browser editor, a successful sample plugin, or a working USB display does not satisfy the whole target.

This ledger began as the **2026-10-11 baseline** and now includes the first native desktop implementation. Status describes inspected code and recorded evidence, not planned work. Update each row when its acceptance evidence exists. The acceptance tests below are proposed project tests; they are not claims that Elgato publishes those tests.

The official [7.6 release notes](https://help.elgato.com/hc/en-us/articles/51156609278737-Elgato-Stream-Deck-7-6-Release-Notes) date the release to 22 September 2026. They include Neo Infobar actions, new plugin integrations, image drag/drop fixes, automatic launch fixes, Website icon refresh and Network Dock fixes. Thus, “full 7.6 parity” cannot be reduced to eight keys and four dials. The attached Stream Deck+ is the first physical acceptance device; other models and external integrations remain explicit gaps until assessed.

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
| UI-01 | Standalone native desktop application; launch without a browser | **Verified subset, C/T/H/U.** Default launch opens an installed native egui window with an in-process Rust controller. Native capture, owned-child shutdown, and real-device settings edits are verified. Desktop menu entry validates; separate menu-click acceptance remains unverified. | Launch installed application from Linux desktop menu. Configure and operate the device entirely within its own window. Capture window/process evidence and user acceptance. |
| UI-02 | Device canvas, key/dial selection, live previews and action assignment | **Verified subset, C/T/H/U.** Native canvas displays eight keys/four dials and real previews. User confirmed native settings/save affects physical input. Replacement, deletion, all-slot geometry and broader keyboard workflows still need end-to-end acceptance. | Native canvas maps every Plus slot accurately; assignment, replacement, deletion, keyboard navigation and persisted state all work on hardware. |
| UI-03 | Action library with categories, search, controller filtering and useful plugin metadata | **Partial, C.** Native action selector filters by controller and exposes installed actions; categories/search/icons remain incomplete. | Search/filter official manifests; key actions cannot be accidentally placed on dial-only slots; categories, icons and disabled/incompatible states are clear. |
| UI-04 | Drag/drop, move/copy/paste, reorder, remove, context menus and reusable action editing | **Partial, C/T.** Multi Action steps support native add/remove/reorder and draft editing. General canvas drag/drop, copy/paste and action-sharing files remain missing. | Exercise internal and file-manager drag/drop; copy across pages/profiles; verify nested data and settings survive. Test undo/recovery as a Linux usability requirement. |
| UI-05 | Action-specific configuration with plugin property inspectors | **Verified subset, C/T.** Native Plugin settings opens a WebKit inspector. Unchanged Counter PI registers; instrumented original control saves into the real plugin. Broader forms/file controls remain unverified. | Open unchanged official property inspector; edit controls, file selectors and dynamic lists; verify visible plugin/hardware response and restart persistence. [PI guide][pi] |
| UI-06 | Titles, typography, image selection, icon/background libraries, preview fidelity | **Partial, C.** Fontdue title rendering and raster backgrounds exist; full style controls and asset libraries do not. | Compare imported title size/style/alignment/color, custom icons and state changes with reference screenshots; verify scaled native UI and hardware rendering. [Key guide][keys], [layouts][layouts] |
| UI-07 | Window/tray lifecycle, startup preference, diagnostics and update/about UI | **Partial, C/T.** Native window close, SIGINT/SIGTERM, owned-child cleanup and failure logging work. Tray, login startup, update UI and instance activation remain missing. | Close/reopen window without interrupting configured background actions; explicitly quit stops only this app's children; test login startup, version display and bounded logs. [Preferences][preferences], [7.6 fixes][release76] |

## Profiles, pages, folders and portable data

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| PR-01 | Read existing `.streamDeckProfile` archives/directories without losing unknown data | **Verified subset, A/T.** Six SDK sample profiles and two shipped Plus profiles import; raw fields, settings, states and images are retained. | Expand corpus to user exports from supported app versions and device types. Compare complete logical trees and assets, including missing plugins and non-ASCII names. [Profiles][profiles] |
| PR-02 | Profile library: create, rename, duplicate, delete, select and bind to device | **Verified subset, C/T.** Native library create/rename/duplicate/delete/select and restart persistence implemented for one Plus device; multi-device bindings remain open. | Perform each operation through native GUI, restart, and verify stable per-profile identities and device associations. [Preferences][preferences] |
| PR-03 | Pages: create/reorder/delete, next/previous/go-to and page indicator | **Verified subset, A/C/T.** Native page CRUD/reorder, next/previous/indicator and swipe navigation implemented. Held controls release before disappearing; stable pinned contexts retain ownership. Serialized go-to settings and physical swipe acceptance remain open. | Import both shipped Plus profiles; change every page with GUI, action and swipe; verify boundary behavior and action disappearance/appearance ordering. [Pages release][pages] |
| PR-04 | Nested folders, return-to-parent and configured auto-exit | **Verified subset, A/T.** Shipped folder/back links navigate, with per-profile folder cursor restoration. Configured auto-exit remains missing. | Traverse nested folders from shipped profiles; return to exact parent/page; check per-folder and per-action auto-exit timers. [Auto folder exit][autoexit] |
| PR-05 | Pinned actions/folders with page-specific overrides | **Verified subset, C/T.** Pins span pages/folders, local overrides win, hidden pins have a dedicated editor, and unpin preserves placements. Mapping the Default page to pin storage remains an inference until a nonempty Windows pin export is obtained. | A pinned slot appears across pages, a page override wins locally, and unpin preserves intended assignments. Do not assume an empty Default page means pins are unsupported. [Pinned actions][pinned] |
| PR-06 | Export profiles and action-sharing files; interoperable import/export round trip | **Partial, A/C/T.** Original-format profile ZIP export preserves source fields, assets and duplicate-page references; both shipped Plus round trips pass. Windows app acceptance and action-sharing files remain unverified. | Linux export imports into official Windows app; Windows export reimports into Linux with equivalent pages, folders, actions, settings and assets. [Profiles][profiles], [sharing][sharing] |
| PR-07 | Backup/restore all profiles and migrations without data loss | **Missing, A/C.** Binary contains `ProfilesV2`, `ProfilesV3` and a migration literal; no migration control flow was recovered. | Restore `.StreamDeckProfilesBackup`; test interrupted migration, old backups, rollback and missing-plugin preservation against real fixtures. [Backups][backups] |
| PR-08 | Smart Profiles switch according to focused application | **Missing, C.** No focus monitoring or application bindings. | On supported Linux desktop sessions, focus configured apps, return to default, and verify editing does not cause disruptive switching. Record compositor-specific restrictions honestly. [Smart Profiles][smart] |
| PR-09 | Plugin-bundled profiles and authorized profile switching | **Verified subset, A/C/T.** Unchanged Lights Out auto-imports its Plus profile and runs real game/image logic. SDK switching validates registration, device, declared profile and page; read-only and persisted bundle bindings tested. | Unchanged official Lights Out plugin installs/activates its declared profile and returns correctly. Validate device/page and manifest profile-access rules. [SDK profiles][profiles] |
| PR-10 | Embedded resources, dependency discovery and repair of missing plugins | **Partial, A/C.** Unknown action UUIDs are retained/inactive. General resource import/export and dependency installation are absent. | Import a 7.1+ profile containing media/resources; install a missing plugin and regain function without recreating action settings. [Resources][resources], [7.1 release][release71] |

## Built-in actions and action composition

The shipped profiles contain real action identifiers and settings; their presence is evidence of a compatibility input, not of our implementation. At this baseline, host dispatch resolves installed plugin actions only. Imported built-ins remain inactive.

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| AC-01 | Open application/file/folder, close application, open URL and background request | **Verified subset, A/C/T.** Native Website/Open editors and Linux argv-based execution; actual local background HTTP success/error and executable launch verified. Browser association, Close and Open Application activation/long-press remain open. | Configure through GUI and import fixtures; verify Linux path/app translation, URL handling and explicit background-request behavior. Unmappable Windows paths remain visible and editable. [System actions][system] |
| AC-02 | Hotkey, hotkey switch, modifiers, text entry and optional Enter | **Partial, A/C/T.** Native shortcut editor, Qt-to-evdev translation, held ownership and exact Plus trigger indices implemented. uinput device creation/removal verified, but event readback skipped for missing event-node read access. Compositor shortcuts, arbitrary layouts, Coalesce, hotkey switch and text remain open. | Exercise a named Linux application under supported X11/Wayland sessions; verify Unicode, modifier release, repeated presses, focus and held keys without stuck input. [System actions][system] |
| AC-03 | Multimedia transport, mute and volume | **Partial, A/C/T.** Original transport enum and composite volume-dial mapping recovered and implemented through Linux media key events. Real audio/player acceptance and external state feedback remain open. | Play/pause/seek where supported, mute and change volume against real media players; verify externally changed state feeds back. [System actions][system] |
| AC-04 | Soundboard playback, looping, overlap/restart, fades, volume and output device | **Verified subset, A/C/T.** Native controls, independent streaming mixer and Pulse/PipeWire transport implement four playback modes, volume, fades, routing and soundboard-wide stop. Captured virtual-sink PCM verifies WAV/embedded MP3, volume and fade envelopes. Audible/physical acceptance, Windows timing/visual equivalence and multichannel downmix remain open. | Play supported audio fixtures in each documented mode, route to selected output, stop/retrigger and restart host safely. Verify captured audio and audible output. [Soundboard][soundboard] |
| AC-05 | Multi Actions, delays, switch/random variants and plugin multi-action context | **Verified subset, A/C/T.** Native ordered-step editor, delays, Switch and Random run through an incremental scheduler. Child SDK payloads, desired states, settings/PI ownership, cancellation and restart identities pass real-process tests; unchanged Hello World runs nested. Physical shortcuts and full Windows workflow equivalence remain open. | Import/reorder sequences, check measured timing and stateful branches, then verify plugin events with a fixture that consumes multi-action state. Inventory switch/random behavior before marking those variants passed. [Multi Actions][multi], [delay controls][delays] |
| AC-06 | Dial Stacks | **Verified subset, A/C/T.** Native editing, selected-child lifecycle, click/held-turn/touch routing, fast navigation and persistence work. Unchanged Layouts instances produce verified PNG changes. Physical acceptance and exact artwork/restore behavior remain open. | Load multiple real dial actions; dial push cycles active action and corresponding touch display; settings persist independently. [Dial Stacks][stacks] |
| AC-07 | Action Trigger adapts key actions to dial directions/press | **Missing, A/C.** Shipped key-adaptor actions are preserved without execution. | Configure three different actions, exercise both rotation directions and press, and verify corresponding plugin key-event semantics. [Action Trigger][trigger] |
| AC-08 | Action Wheel and Key Logic/Key Stack behaviors | **Missing/Unresolved, A/C.** Shipped `keys.logic`/`keys.stack` descriptors exist; semantics are not fully investigated. | Implement documented wheel selection/activation and record actual 7.6 Key Logic/Stack reference behavior before creating differential tests. [Action Wheel][wheel], [7.4 fixes][release74] |
| AC-09 | Device brightness, sleep/screensaver and profile/page navigation actions | **Partial, A/C/T.** Profile/page/folder navigation implemented and process-tested. Brightness, sleep and screensaver remain missing. | Execute user-configured actions on hardware, verify ranges/state restoration, and ensure navigation targets the correct device. [Preferences][preferences], [screensaver][screensaver] |
| AC-10 | Built-in/tutorial onboarding and meaningful errors for unavailable actions | **Partial, A/C/T.** Native editor explains unavailable imported actions and retains their data; tutorial HTML registers. Broader tutorial screens and every default-profile action remain open. | Import untouched default Plus profile; every action either works or has an actionable visible compatibility error. Run tutorial screens through the supported browser runtime. [Artifact evidence](../../evidence/elgato-7.6/findings.md) |

## Plugin compatibility and rendering contract

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| PL-01 | Node plugins, manifest/runtime version selection and lifecycle | **Verified subset, T.** Unchanged Counter and Layouts register using installed Node 20. Generic CodePath only; installed runtime search is limited. | Test Node 20/24, version mismatch, startup failure, shutdown/restart and plugin supervision; respect manifest versions and platform-specific paths. [Manifest][manifest], [environment][environment] |
| PL-02 | Legacy HTML plugin runtime | **Verified subset, A/C/T.** Native hidden HTML runtime registers unchanged shipped tutorial; generated HTML backend/PI fixture exchanges real SDK messages. Full tutorial navigation and background-media behavior remain open. | Run untouched tutorial HTML with the expected injected connection callback; verify browser networking, local resources, lifecycle and actual responses. [Artifact findings](../../evidence/elgato-7.6/findings.md) |
| PL-03 | Native plugins and OS-dependent modules | **Missing/Unresolved, C.** Native executables and platform bindings do not run. | Inventory Windows/macOS-dependent plugins, provide a supported runtime/porting path, and test real native binaries or Linux ports against the same contract. Explicit per-plugin limitations remain until resolved. [WebSocket API][ws], [manifest][manifest] |
| PL-04 | Property-inspector browser, registration and bidirectional messaging | **Verified subset, C/T.** Native inspector windows register, route messages/settings, enforce per-action context binding and send appearance/disappearance. Original Counter PI registers; its unchanged control code saves via explicit instrumentation. | Unchanged official inspector registers, receives action/settings metadata, exchanges `sendToPlugin`/`sendToPropertyInspector`, and gets appearance/disappearance events. [PI guide][pi] |
| PL-05 | Device/action lifecycle and physical input events | **Verified subset, C/T/H.** Registration, device-connect and key/dial events work for samples; broader lifecycle incomplete. | Contract fixtures cover disconnect/reconnect, page/folder transitions, simultaneous instances, title changes and touch coordinates; then repeat physical paths. [WebSocket API][ws] |
| PL-06 | Action/global settings, correlation IDs and persistent isolation | **Verified subset, T.** Sample settings survive restart; global storage and correlated replies exist in code. | Test SDK v1/v2/v3 clients, PI-origin updates, concurrent gets, multi-plugin isolation, malformed settings and crash-safe persistence. [Settings][settings], [SDK 3 changes][sdk3] |
| PL-07 | Title/image/state commands and separate hardware/software targets | **Partial, C.** Current state and combined target work; inactive-state/separate-target requests are rejected. | Verify all states and targets, user-title precedence, resets and plugin overrides before/after state changes against reference output. [Keys][keys], [WebSocket API][ws] |
| PL-08 | Image formats, animation and resource path behavior | **Partial, C/T.** PNG/JPEG/GIF and SVG file/data-URI rendering work; SVG transparency is checked. Animation and WebP remain missing. | Test file/data-URI icons, SVG and animated assets, scaling/transparency, frame timing and bounded memory with real devices. [Keys][keys], [6.0.1 release][release601] |
| PL-09 | All built-in feedback layouts and custom JSON layouts | **Verified subset, C/T.** All six built-ins and four official custom layouts run unchanged with real preview-pixel checks. Incremental properties, SVG, clipping/layers/gradients work. Exact Windows text/bar raster parity remains unverified. | Run every unchanged Layouts sample action; compare text, pixmap, bars, ranges, z-order, clipping, opacity and incremental updates. [Layouts][layouts] |
| PL-10 | Visual alerts, success feedback and trigger descriptions | **Partial, C.** `showAlert`/`showOk` only log; trigger descriptions are accepted without UI effect. | Observe correct temporary overlays and restoration on keys/dials; inspector descriptions update when requested. [WebSocket API][ws], [dials][dials] |
| PL-11 | Embedded plugin resources | **Missing, C.** Events use empty resources; no get/set resource command implementation. | Save/update resources through unchanged plugin/PI, export profile, import elsewhere and reopen the file successfully. [Resources][resources] |
| PL-12 | App monitoring, wake notifications, deep links, openURL and localization | **Missing/Partial, C.** `logMessage` exists; external/system callbacks and plugin localization are absent. | Trigger app launch/exit, suspend/resume and routed URI callbacks; test translated metadata/UI and external browser operations. [Monitoring][monitoring], [deep links][deeplinks], [environment][environment] |
| PL-13 | Plugin crash handling, timeouts, upgrades and removal while actions remain | **Partial, C.** Owned child shutdown exists; restart supervision and replacement policies absent. | Crash one plugin without freezing host/other plugins; recover with action settings intact; test repeated failures and upgrade rollback. [Environment][environment], [6.0.1 fixes][release601] |

## Installation, Marketplace and content

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| IN-01 | Install `.streamDeckPlugin` archives as inert validated files | **Verified subset, T.** `package.rs` checks paths/UUID/size, stages privately and atomically refuses overwrite. | Native file-picker/drop integration, success/error feedback and newly installed action discovery. Retain malicious archive and collision tests. [Plugin guide][plugins] |
| IN-02 | Plugin discovery, enable/disable, update, uninstall and dependency handling | **Partial, C.** CLI chooses explicit directories; existing install cannot be overwritten. | Manage plugins from native UI; preserve profile references/settings across removal/reinstall; handle versions and missing dependencies visibly. [Plugin guide][plugins], [backups][backups] |
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
| HW-06 | Brightness, naming, sleep, lock behavior, screensaver and wake | **Missing, C.** Firmware query exists; persistent device controls not exercised. | Set values through GUI, observe physical device; lock/unlock and timer wake restore correct profile without spurious actions. [Preferences][preferences], [screensaver][screensaver] |
| HW-07 | USB reconnect, suspend/resume and startup permission handling | **Partial, C.** Disconnect is logged; user must restart host. | Unplug/replug same and different serials, suspend/resume, and start without access; recover automatically with actionable permission UI and intact state. [Device protocol][hidgeneral] |
| HW-08 | Multiple devices and cross-device profile actions | **Missing, C.** One connected device per host instance. | Operate two devices concurrently with independent profiles; switch one device from another and test removal while the other remains active. [Multiple devices][devices] |
| HW-09 | Other models, Neo Infobar, + XL, Mobile/virtual devices and Network Dock | **Missing/Unresolved, D/C.** HID implementation targets Plus only. | Inventory 7.6 supported surfaces and transport contracts; implement each, then run real hardware/network/mobile acceptance. Simulated layouts alone are insufficient. [7.6 release][release76], [7.1 release][release71], [layouts][layouts] |
| HW-10 | Firmware inspection/update workflow | **Partial, C/H.** Current firmware can be read; updater absent. | Separate update eligibility from write operations; validate correct model/image and recovery using authorized devices. Retain explicit unresolved status until a safe supported update path exists. [Preferences][preferences], [7.1 release][release71] |
| OS-01 | Native package, launcher, autostart and clean uninstall | **Partial, C/T/H/U.** Per-user optimized binary and validated desktop launcher installed, settings migrated with original retained. Clean-machine uninstall/autostart remain open. | Install/uninstall on a clean supported Linux system; launcher works outside repository; data migration/retention and login lifecycle are verified. User desktop requirement; [7.6 autostart fix][release76]. |
| OS-02 | Audio routing and per-app/input/output controls | **Missing/Unresolved, A/C.** Shipped Plus profile includes volume-controller actions; no Linux audio integration. | Imported actions control real Linux input/output/app streams with equivalent mute/volume feedback and reconnect behavior. A placeholder volume number is insufficient. [7.1 plugin note][release71], [soundboard][soundboard] |
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
