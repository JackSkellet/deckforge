# Icon packs

Open **Manage icon packs…** in the sidebar to manage the library without assigning
an action. Open an action's **Appearance → Icon library…** to choose its artwork.
Search matches icon names, tags and filenames. Choose an icon, then Save the
action; selection remains a draft until Save and supports Undo/Redo. The selected
state and edit callback are captured when opening the library. Import pack accepts
`.streamDeckIconPack` ZIPs with one `.sdIconPack` root; Import folder accepts an
existing `.sdIconPack` directory. Existing IDs are never overwritten.

The library shows60 results at a time with Show more, keyboard-accessible buttons,
lazy thumbnails and visible import errors. Thumbnail requests share the existing
four-in-flight image queue. A checkerboard makes both black and white transparent
icons visible. Failed thumbnails have a fallback; normal appearance validation
still applies to selection. Library/import/cancel buttons never submit the outer
action form. Native file-picker interaction is not automated in acceptance tests.

## Original format and evidence

The supplied Stream Deck7.6 Key Creator plugin archive contains six packs:
Elgato Icons Black/White, Legacy Icons, and Logo Icons Black/Color/White. There are
2,361 image files. Legacy has214 metadata rows and219 files; unlisted images get
filename labels. Black/White logo icons.json files have UTF-8 BOMs. Those BOMs
are accepted without rewriting the source bytes.

Native static evidence from AppExecutable SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`:

- `0x1401abfd0`: derive identity from the `.sdIconPack` folder name.
- `0x1401ad120`: refuse duplicate IDs and install under `<id>.sdIconPack`.
- `0x140346780` / `0x140345ee0`: parse manifest; read Author, Name, Icon and Version
  as strings. Unknown manifest fields are retained as source bytes, not invented
  constraints on pack identity.
- `0x140396e60`: load optional icons.json from the pack root.
- `0x1403469e0` / `0x140346220`: metadata is an array; read path/name and tags helper.

Shipped data confirms lowercase path/name/tags entries and paths relative to
`icons/`. Description, licensing and other original files stay intact. Evidence:
`../evidence/icon-packs/observations.json` and `rea-bundle.json` (160 records).
No original vendor executable was run. These installed-folder observations do
not prove every Marketplace download/envelope variant.

## Installation and ownership

`src/icon_pack.rs` loads, installs, lists and resolves assets. ZIP parsing reuses
`package::read_archive_entries`, including central-directory duplicate checks,
path/type checks and file/directory collision rejection. Bounds are10,000 entries,
32MiB per file and128MiB expanded. Directory imports use the same limits and
anchored no-follow regular-file reads; links/special files are rejected. FIFO ZIP
sources are rejected without blocking. Installation stages privately, validates
manifest/metadata, clears executable bits and atomically renames without overwrite.
Installed pack roots are mode700 and files mode600. Unknown files remain inert.

Managed packs live in `<state>/icon-packs`. Catalog errors are isolated per pack.
Metadata duplicates/traversal fail explicitly. Supported image files are enumerated
independently of metadata, so unlisted images remain selectable. Thumbnail paths
must resolve to a catalog entry and use the existing bounded decoder, with no
external SVG resource loading. Pack IO happens outside the host mutex.

Selecting an icon immediately retains its original bytes in the existing owned
appearance-assets cache; Save later publishes the action edit. Removing or
replacing the source pack cannot break either that pending draft or saved artwork.
The same content hash/basename storage makes Save idempotent and preserves exact
profile-export bytes. Cancelled selections may leave deduplicated cache files;
automatic cache cleanup is not part of this change. Importing packs does not launch code or alter profiles.
The protected-manifest verifier can be used with the configured reference, but
actual protected icon-pack acceptance remains unverified.

The CLI supports `icon-pack inspect <folder>`, `icon-pack list <directory>`, and
`icon-pack install <archive-or-folder> --destination <directory>`; `--reference-app`
can supply the existing verifier reference. These commands do not start the host
or open USB. Native UI uses the same Controller API and parser.

## Visibility, replacement and recovery

Hide/Show is a persistent per-pack preference outside original manifests.
Hidden packs appear when Show hidden packs is checked or when searching, matching
the shipped icon editor. Pinned packs cannot be removed; a pinned pack without
canhide also cannot be hidden. The original JS and CSS explicitly hide its
uninstall control and lock that visibility checkbox. Boolean and string-boolean
manifest flags are accepted. Pinned packs sort before other packs.

Update pack stages and validates the selected file, then shows current/replacement
versions and icon counts. Applying restages the source and checks its reviewed
content hash plus the current installed filesystem revision. Changed sources or
targets fail before publication. Remove shows the selected pack before archiving
it. Archived versions list removal/replacement time and can be reviewed/restored;
a restore over a current installation retains that displaced version too.

All cooperating operations use a private nonblocking library lock. Replacement
reuses the existing atomic Linux directory-exchange helper; removal atomically
moves the pack. A private recovery record is written and flushed before the move
or exchange. Directory identities distinguish committed predecessors from
uncommitted candidates; content hashes validate an archive again before restore.
Recovery directories use explicit mode700, records/files mode600. Retained
archives are not automatically deleted. Newly created record timestamps include
nanoseconds for deterministic newest-first ordering within one second.

Visibility is stored under `icon-packs/preferences/visibility/hidden.json`.
Backups live in retained `.deckforge-icons-*` transaction directories beneath the
library, with `transaction.json` and the prior `.sdIconPack` directory. Bad recovery
records become catalog errors without discarding active packs. These checks cover
observed filesystem changes/cooperating operations; they are not a guarantee
against adversarial concurrent filesystem replacement or power loss.

Source evidence: `../evidence/icon-pack-management/observations.json` records
original JS/CSS hashes and exact pinned/visibility/uninstall/search contexts.
The REA bundle has162 records. Its one-module JavaScript graph analysis is partial:
the semantic worker exhausted its heap. The direct source predicates/CSS and
controlled Linux tests support the behavior described here; no complete original
application reconstruction or Windows runtime acceptance is claimed.

## Live library refresh

The open library follows local filesystem changes without Reload. The native
bridge owns an inotify watcher and includes an icon_library revision/error in
its snapshots. It watches the library parent (for late creation), active pack
directories recursively, visibility preferences and recovery metadata. Reads,
lock-file traffic, profile saves and appearance caches do not invalidate it.
Directories are watched before enumeration; identities retarget watches after
atomic replacement. Overflow or an exhausted per-tick read budget recreates the
inotify instance and rebuilds the watch set, including lost removal events.

The existing bridge worker drains notifications nonblocking. Bursts coalesce with
100ms quiet time and a1s maximum delay; no new thread or icon-byte polling is used.
Directory watches are bounded at4096, reads at16×32KiB per tick, and watcher errors
retry after5s. These are implementation bounds, not guaranteed end-to-end latency.

The frontend serializes automatic catalog reads, defers them during active work,
ignores old replies and retries transient contention without clearing the last
catalog. Searches and valid selection remain. A removed selection resets to All
packs. Pending reviews keep their original approval baseline; if the target changes,
Apply is disabled and the user is asked to review again. Observer/read failures
are visible, and Reload remains available.

These are local Linux notification semantics. Remote filesystem changes, mmap
writes and mount-over-watch cases may not generate the required notifications;
Use Reload to read the catalog in those cases. The inode/recursive-watch/overflow behavior follows
the [Linux inotify documentation](https://man7.org/linux/man-pages/man7/inotify.7.html).
Adversarial filesystem races and universal network-filesystem acceptance are not
claimed.

Four actual-watcher/parser tests cover late creation, nested writes (including an
open writer), visibility, ignored reads/profile files, replacement retargeting,
recovery records, observed symlinks and overflow with a deliberately lost watch
removal event. Frontend tests cover notification coalescing, late replies, busy
read retry and preserved stale review/search. `tests/native_icon_pack_watch_interop.py`
keeps the real library open while another process installs packs, exchanges an
installed directory, edits metadata/pixels, removes a pack and atomically changes
visibility files. All changes appear without Reload; the profile graph stays
unchanged. Native management regression passes with this watcher active. Reports
and capture: `target/pack-watch/`. No USB is used.

## Verification and remaining work

Generated tests cover folder/ZIP installation, exact bytes, names/tags/unlisted
icons, no-overwrite behavior, thumbnail output, metadata errors, path traversal,
symlinks and FIFO sources. An explicit installer-artifact test installs all six
real packs and compares every file byte. The shared plugin parser's generated and
original Layouts-archive tests pass. Frontend tests cover tag filtering, asset
resolution, import cancellation/refresh and no unintended parent-form submission.

`tests/native_icon_pack_interop.py` installs the original Color Logo pack through
the native bridge, searches a tag and renders its SVG in the real library. It
selects/saves/undoes/redoes the icon, restarts after deleting original and installed
pack sources, then verifies exact SVG bytes in exported profiles. Reports and
visually checked captures: `target/icon-packs/native/`. No USB is involved.

Desktop file opening and drops now install icon packs through the same validator;
see [FILE_OPEN.md](FILE_OPEN.md). Management tests cover pinned/visibility/stale
checks, selected-byte retention, replacement/recovery, changed-source/archive
rejection, nonblocking lock contention and interrupted exchange layouts. The
shared plugin-update tests pass. Frontend tests cover review/cancel/apply, hidden
search, pinned rules, global empty-profile management and parent-form guards.
`tests/native_icon_pack_management_interop.py` proves native hide/search/remove/
restore controls, the real reviewed replacement API and refreshed thumbnails,
pending-icon Save after removal, independent visibility and recovery after
restart. The file picker is covered by frontend dispatch tests, not OS dialog
automation. Original selection/Save/Undo/restart/export regression also passes.
Reports: `target/pack-management/`.

Marketplace pack handoff, background/screensaver catalogs, the full Key Creator composition
workflow, protected download variants and physical-device acceptance remain open.
This is progress on IN-04, not full feature parity. Original assets installed for
this user remain local; the executable does not embed or redistribute the packs.
