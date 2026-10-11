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
| UI-04 | Drag/drop, move/copy/paste, reorder, remove, context menus and reusable action editing | **Missing, C.** No native editing workflow. [Action sharing][sharing] confirms import/export workflows. | Exercise internal and file-manager drag/drop; copy across pages/profiles; verify nested data and settings survive. Test undo/recovery as a Linux usability requirement. |
| UI-05 | Action-specific configuration with plugin property inspectors | **Missing, C.** JSON settings textarea is the available editor. | Open unchanged official property inspector; edit controls, file selectors and dynamic lists; verify visible plugin/hardware response and restart persistence. [PI guide][pi] |
| UI-06 | Titles, typography, image selection, icon/background libraries, preview fidelity | **Partial, C.** Fontdue title rendering and raster backgrounds exist; full style controls and asset libraries do not. | Compare imported title size/style/alignment/color, custom icons and state changes with reference screenshots; verify scaled native UI and hardware rendering. [Key guide][keys], [layouts][layouts] |
| UI-07 | Window/tray lifecycle, startup preference, diagnostics and update/about UI | **Missing, C.** Foreground CLI and logs exist; closing a native window is undefined. | Close/reopen window without interrupting configured background actions; explicitly quit stops only this app's children; test login startup, version display and bounded logs. [Preferences][preferences], [7.6 fixes][release76] |

## Profiles, pages, folders and portable data

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| PR-01 | Read existing `.streamDeckProfile` archives/directories without losing unknown data | **Verified subset, A/T.** Six SDK sample profiles and two shipped Plus profiles import; raw fields, settings, states and images are retained. | Expand corpus to user exports from supported app versions and device types. Compare complete logical trees and assets, including missing plugins and non-ASCII names. [Profiles][profiles] |
| PR-02 | Profile library: create, rename, duplicate, delete, select and bind to device | **Missing, C.** A single CLI-selected current page becomes active slots. | Perform each operation through native GUI, restart, and verify stable per-profile identities and device associations. [Preferences][preferences] |
| PR-03 | Pages: create/reorder/delete, next/previous/go-to and page indicator | **Partial, A/C.** Page metadata and encoded UUID folders parse; active page navigation is absent. | Import both shipped Plus profiles; change every page with GUI, action and swipe; verify boundary behavior and action disappearance/appearance ordering. [Pages release][pages] |
| PR-04 | Nested folders, return-to-parent and configured auto-exit | **Partial, A/T.** Folder UUID references resolve in parser tests; host does not execute folder navigation. | Traverse nested folders from shipped profiles; return to exact parent/page; check per-folder and per-action auto-exit timers. [Auto folder exit][autoexit] |
| PR-05 | Pinned actions/folders with page-specific overrides | **Missing, C.** Default page parses but no inheritance/rendering rule is implemented. | A pinned slot appears across pages, a page override wins locally, and unpin preserves intended assignments. Do not assume an empty Default page means pins are unsupported. [Pinned actions][pinned] |
| PR-06 | Export profiles and action-sharing files; interoperable import/export round trip | **Missing, C.** Import is read-only; session JSON is not an Elgato export. | Linux export imports into official Windows app; Windows export reimports into Linux with equivalent pages, folders, actions, settings and assets. [Profiles][profiles], [sharing][sharing] |
| PR-07 | Backup/restore all profiles and migrations without data loss | **Missing, A/C.** Binary contains `ProfilesV2`, `ProfilesV3` and a migration literal; no migration control flow was recovered. | Restore `.StreamDeckProfilesBackup`; test interrupted migration, old backups, rollback and missing-plugin preservation against real fixtures. [Backups][backups] |
| PR-08 | Smart Profiles switch according to focused application | **Missing, C.** No focus monitoring or application bindings. | On supported Linux desktop sessions, focus configured apps, return to default, and verify editing does not cause disruptive switching. Record compositor-specific restrictions honestly. [Smart Profiles][smart] |
| PR-09 | Plugin-bundled profiles and authorized profile switching | **Missing, C.** Manifest `Profiles` is not acted upon; `switchToProfile` is unsupported. | Unchanged official Lights Out plugin installs/activates its declared profile and returns correctly. Validate device/page and manifest profile-access rules. [SDK profiles][profiles] |
| PR-10 | Embedded resources, dependency discovery and repair of missing plugins | **Partial, A/C.** Unknown action UUIDs are retained/inactive. General resource import/export and dependency installation are absent. | Import a 7.1+ profile containing media/resources; install a missing plugin and regain function without recreating action settings. [Resources][resources], [7.1 release][release71] |

## Built-in actions and action composition

The shipped profiles contain real action identifiers and settings; their presence is evidence of a compatibility input, not of our implementation. At this baseline, host dispatch resolves installed plugin actions only. Imported built-ins remain inactive.

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| AC-01 | Open application/file/folder, close application, open URL and background request | **Missing, A/C.** Shipped fixtures contain system open/website actions. | Configure through GUI and import fixtures; verify Linux path/app translation, URL handling and explicit background-request behavior. Unmappable Windows paths remain visible and editable. [System actions][system] |
| AC-02 | Hotkey, hotkey switch, modifiers, text entry and optional Enter | **Missing, A/C.** Original Windows key codes/settings are preserved. | Exercise a named Linux application under supported X11/Wayland sessions; verify Unicode, modifier release, repeated presses, focus and held keys without stuck input. [System actions][system] |
| AC-03 | Multimedia transport, mute and volume | **Missing, A/C.** Multimedia UUID/settings exist in shipped Plus profiles. | Play/pause/seek where supported, mute and change volume against real media players; verify externally changed state feeds back. [System actions][system] |
| AC-04 | Soundboard playback, looping, overlap/restart, fades, volume and output device | **Missing, A/C.** Shipped profiles reference `soundboard.playaudio`; no audio engine exists. | Play supported audio fixtures in each documented mode, route to selected output, stop/retrigger and restart host safely. Verify captured audio and audible output. [Soundboard][soundboard] |
| AC-05 | Multi Actions, delays, switch/random variants and plugin multi-action context | **Missing, A/C.** Shipped fixtures include routine/delay UUIDs; outgoing events always say `isInMultiAction:false`. | Import/reorder sequences, check measured timing and stateful branches, then verify plugin events with a fixture that consumes multi-action state. Inventory switch/random behavior before marking those variants passed. [Multi Actions][multi], [delay controls][delays] |
| AC-06 | Dial Stacks | **Missing, A/C.** `com.elgato.streamdeck.dial.stack` appears in shipped profiles. | Load multiple real dial actions; dial push cycles active action and corresponding touch display; settings persist independently. [Dial Stacks][stacks] |
| AC-07 | Action Trigger adapts key actions to dial directions/press | **Missing, A/C.** Shipped key-adaptor actions are preserved without execution. | Configure three different actions, exercise both rotation directions and press, and verify corresponding plugin key-event semantics. [Action Trigger][trigger] |
| AC-08 | Action Wheel and Key Logic/Key Stack behaviors | **Missing/Unresolved, A/C.** Shipped `keys.logic`/`keys.stack` descriptors exist; semantics are not fully investigated. | Implement documented wheel selection/activation and record actual 7.6 Key Logic/Stack reference behavior before creating differential tests. [Action Wheel][wheel], [7.4 fixes][release74] |
| AC-09 | Device brightness, sleep/screensaver and profile/page navigation actions | **Missing, A/C.** `system.keybrightness`, `system.sleep` and page UUIDs exist in installer profiles. | Execute user-configured actions on hardware, verify ranges/state restoration, and ensure navigation targets the correct device. [Preferences][preferences], [screensaver][screensaver] |
| AC-10 | Built-in/tutorial onboarding and meaningful errors for unavailable actions | **Partial, A/C.** Missing actions are logged; bundled tutorial is an unrun HTML plugin. | Import untouched default Plus profile; every action either works or has an actionable visible compatibility error. Run tutorial screens through the supported browser runtime. [Artifact evidence](../../evidence/elgato-7.6/findings.md) |

## Plugin compatibility and rendering contract

| ID | Required behavior | Baseline status and evidence | Evidence needed to pass |
| --- | --- | --- | --- |
| PL-01 | Node plugins, manifest/runtime version selection and lifecycle | **Verified subset, T.** Unchanged Counter and Layouts register using installed Node 20. Generic CodePath only; installed runtime search is limited. | Test Node 20/24, version mismatch, startup failure, shutdown/restart and plugin supervision; respect manifest versions and platform-specific paths. [Manifest][manifest], [environment][environment] |
| PL-02 | Legacy HTML plugin runtime | **Missing, A/C.** Node-only launcher rejects HTML. Shipped tutorial declares `CodePath: app.html`. | Run untouched tutorial HTML with the expected injected connection callback; verify browser networking, local resources, lifecycle and actual responses. [Artifact findings](../../evidence/elgato-7.6/findings.md) |
| PL-03 | Native plugins and OS-dependent modules | **Missing/Unresolved, C.** Native executables and platform bindings do not run. | Inventory Windows/macOS-dependent plugins, provide a supported runtime/porting path, and test real native binaries or Linux ports against the same contract. Explicit per-plugin limitations remain until resolved. [WebSocket API][ws], [manifest][manifest] |
| PL-04 | Property-inspector browser, registration and bidirectional messaging | **Missing, C.** No `registerPropertyInspector` session or HTML UI embedding. | Unchanged official inspector registers, receives action/settings metadata, exchanges `sendToPlugin`/`sendToPropertyInspector`, and gets appearance/disappearance events. [PI guide][pi] |
| PL-05 | Device/action lifecycle and physical input events | **Verified subset, C/T/H.** Registration, device-connect and key/dial events work for samples; broader lifecycle incomplete. | Contract fixtures cover disconnect/reconnect, page/folder transitions, simultaneous instances, title changes and touch coordinates; then repeat physical paths. [WebSocket API][ws] |
| PL-06 | Action/global settings, correlation IDs and persistent isolation | **Verified subset, T.** Sample settings survive restart; global storage and correlated replies exist in code. | Test SDK v1/v2/v3 clients, PI-origin updates, concurrent gets, multi-plugin isolation, malformed settings and crash-safe persistence. [Settings][settings], [SDK 3 changes][sdk3] |
| PL-07 | Title/image/state commands and separate hardware/software targets | **Partial, C.** Current state and combined target work; inactive-state/separate-target requests are rejected. | Verify all states and targets, user-title precedence, resets and plugin overrides before/after state changes against reference output. [Keys][keys], [WebSocket API][ws] |
| PL-08 | Image formats, animation and resource path behavior | **Partial, C.** PNG/JPEG/GIF decoding exists; no animation scheduler or SVG/WebP rendering. | Test file/data-URI icons, SVG and animated assets, scaling/transparency, frame timing and bounded memory with real devices. [Keys][keys], [6.0.1 release][release601] |
| PL-09 | All built-in feedback layouts and custom JSON layouts | **Partial, T.** `$B1` title/value/indicator subset works; other layouts rejected. | Run every unchanged Layouts sample action; compare text, pixmap, bars, ranges, z-order, clipping, opacity and incremental updates. [Layouts][layouts] |
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
| HW-04 | Touch tap/hold and swipe navigation | **Partial, C/T.** Packets decode and tap routing exists. Swipe is logged as unimplemented; full touch acceptance unproven. | Physically tap/hold all four segments and swipe between real pages. Compare local coordinates, direction, hold semantics and visible effects. [Touch guide][touch] |
| HW-05 | Per-action/dial/whole-strip backgrounds | **Partial, C.** Fixed renderer background and imported key images; full touch background layers absent. | Configure and import all three official touch-strip layers; verify composition and retained settings. [Touch guide][touch] |
| HW-06 | Brightness, naming, sleep, lock behavior, screensaver and wake | **Missing, C.** Firmware query exists; persistent device controls not exercised. | Set values through GUI, observe physical device; lock/unlock and timer wake restore correct profile without spurious actions. [Preferences][preferences], [screensaver][screensaver] |
| HW-07 | USB reconnect, suspend/resume and startup permission handling | **Partial, C.** Disconnect is logged; user must restart host. | Unplug/replug same and different serials, suspend/resume, and start without access; recover automatically with actionable permission UI and intact state. [Device protocol][hidgeneral] |
| HW-08 | Multiple devices and cross-device profile actions | **Missing, C.** One connected device per host instance. | Operate two devices concurrently with independent profiles; switch one device from another and test removal while the other remains active. [Multiple devices][devices] |
| HW-09 | Other models, Neo Infobar, + XL, Mobile/virtual devices and Network Dock | **Missing/Unresolved, D/C.** HID implementation targets Plus only. | Inventory 7.6 supported surfaces and transport contracts; implement each, then run real hardware/network/mobile acceptance. Simulated layouts alone are insufficient. [7.6 release][release76], [7.1 release][release71], [layouts][layouts] |
| HW-10 | Firmware inspection/update workflow | **Partial, C/H.** Current firmware can be read; updater absent. | Separate update eligibility from write operations; validate correct model/image and recovery using authorized devices. Retain explicit unresolved status until a safe supported update path exists. [Preferences][preferences], [7.1 release][release71] |
| OS-01 | Native package, launcher, autostart and clean uninstall | **Missing, C.** Workspace executable and shell launcher only. | Install/uninstall on a clean supported Linux system; launcher works outside repository; data migration/retention and login lifecycle are verified. User desktop requirement; [7.6 autostart fix][release76]. |
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
