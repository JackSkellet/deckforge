import { useEffect, useId, useRef, useState } from "react";
import { pick, request } from "./bridge";
import { Check, Field } from "./Fields";
import { Modal } from "./Panels";
import { useIconCatalog } from "./useIconCatalog";
interface Icon { path: string; name: string; tags: string[]; }
interface Pack { id: string; name: string; author: string; version: string; icons: Icon[]; warnings: string[];
  revision: string; pinned: boolean; can_hide: boolean; hidden: boolean; }
interface Archive { token: string; id: string; name: string; version: string; icons: number; kind: string; created: number; }
interface Summary { name: string; version: string; icons: number; }
interface Review { id: string; source: string; expected_revision: string | null; candidate_digest: string; current: Summary | null; replacement: Summary; }
type Change = { kind: "remove"; pack: Pack } | { kind: "replace"; review: Review };
interface Catalog { packs: Pack[]; errors: string[]; archives?: Archive[]; }
function Thumbnail({ id, revision, icon }: { id: string; revision: string; icon: Icon }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let live = true;
    const load = () => void request<string>("icon_packs", { operation: "thumbnail", id, revision, path: icon.path })
      .then((value) => { if (live && value.startsWith("data:image/png;base64,")) setSrc(value); })
      .catch((e) => { if (live) setError(String(e)); });
    let observer: IntersectionObserver | undefined;
    if (typeof IntersectionObserver === "undefined") load();
    else {
      observer = new IntersectionObserver((entries) => {
        if (entries.some((e) => e.isIntersecting)) { observer?.disconnect(); load(); }
      });
      if (ref.current) observer.observe(ref.current);
    }
    return () => { live = false; observer?.disconnect(); };
  }, [id, revision, icon.path]);
  return <span className="pack-icon-preview" ref={ref} aria-hidden="true" title={error || undefined}>
    {src ? <img src={src} alt="" /> : <span>{error ? "?" : "…"}</span>}
  </span>;
}
export function IconLibrary({ onChoose, onClose }: { onChoose?: (path: string) => void; onClose: () => void }) {
  const archiveDescription = useId();
  const [pack, setPack] = useState("");
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(60);
  const [busy, setBusy] = useState(false);
  const { catalog, refresh, watchError } = useIconCatalog<Catalog>(busy);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [showHidden, setShowHidden] = useState(false);
  const [change, setChange] = useState<Change | null>(null);
  const [archiveLimit, setArchiveLimit] = useState(20);
  const selected = catalog?.packs.find((p) => p.id === pack);
  useEffect(() => setLimit(60), [pack, query]);
  async function install(directory: boolean) {
    setBusy(true); setError("");
    try {
      const path = await pick(directory ? "directory" : "icon_pack");
      if (!path) return;
      const installed = await request<Pack>("icon_packs", { operation: "install", path });
      await refresh(); setPack(installed.id); setQuery(""); setNotice(`Imported ${installed.name}`);
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  async function choose(selected: Pack, icon: Icon) {
    if (!onChoose) return;
    setBusy(true); setError("");
    try {
      const path = await request<string>("icon_packs", { operation: "asset", id: selected.id, revision: selected.revision, path: icon.path });
      onChoose(path); onClose();
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  async function visibility() {
    if (!selected) return; setBusy(true); setError("");
    try {
      await request("icon_packs", { operation: "hide", id: selected.id, revision: selected.revision, expected_hidden: selected.hidden, hidden: !selected.hidden });
      await refresh(); if (!selected.hidden && !showHidden) setPack("");
      setNotice(`${selected.hidden ? "Showing" : "Hidden"}: ${selected.name}`);
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  async function replace(archive?: Archive) {
    if (!archive && !selected) return; setBusy(true); setError("");
    try {
      let review: Review;
      if (archive) review = await request("icon_packs", { operation: "review_restore", token: archive.token });
      else {
        const path = await pick("icon_pack"); if (!path) return;
        review = await request("icon_packs", { operation: "review", id: selected!.id, path });
      }
      setChange({ kind: "replace", review });
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  async function applyChange() {
    if (!change || reviewStale) return; setBusy(true); setError("");
    try {
      if (change.kind === "remove") {
        await request("icon_packs", { operation: "remove", id: change.pack.id, revision: change.pack.revision });
        await refresh(); setPack(""); setNotice(`Removed ${change.pack.name}. It is available under Archived versions.`);
      } else {
        await request("icon_packs", { operation: "apply", review: change.review });
        const next = await refresh(); const restored = next.packs.find((p) => p.id === change.review.id);
        if (restored?.hidden) setShowHidden(true);
        setPack(change.review.id); setQuery(""); setNotice(`Installed ${change.review.replacement.name} ${change.review.replacement.version}`);
      }
      setChange(null);
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  const reviewed = change?.kind === "remove" ? change.pack.id : change?.review.id;
  const reviewedRevision = change?.kind === "remove" ? change.pack.revision : change?.review.expected_revision;
  const reviewStale = !!change && !!catalog && (catalog.packs.find((p) => p.id === reviewed)?.revision ?? null) !== reviewedRevision;
  useEffect(() => {
    if (catalog && pack && !catalog.packs.some((p) => p.id === pack)) setPack("");
  }, [catalog, pack]);
  const search = query.trim().toLowerCase();
  // The shipped editor includes hidden packs in search results.
  const available = (catalog?.packs || []).filter((p) => !p.hidden || showHidden || !!search);
  useEffect(() => {
    if (selected?.hidden && !showHidden && !search) setPack("");
  }, [selected?.hidden, showHidden, search]);
  const matches = available.filter((p) => !pack || p.id === pack).flatMap((p) =>
    p.icons.filter((i) => [i.name, i.path, ...i.tags].some((s) => s.toLowerCase().includes(search))).map((icon) => ({ pack: p, icon })),
  );
  return <Modal title="Icon library" wide closeDisabled={busy} onClose={onClose}>
    <div className="icon-library" onKeyDown={(event) => {
      if (event.key === "Enter" && event.target instanceof HTMLInputElement && event.target.type === "text") {
        event.preventDefault(); event.stopPropagation();
      }
    }}>
      <p className="hint">{onChoose ? "Choose an icon for this action. Save the action to apply it." : "Manage packs here. Select an action and open its Icon library to choose artwork."}</p>
      {!change && <div className="row"><button type="button" disabled={busy} onClick={() => void install(false)}>Import pack…</button>
        <button type="button" disabled={busy} onClick={() => void install(true)}>Import folder…</button>
        <button type="button" disabled={busy} onClick={() => { setError(""); void refresh().catch((e) => setError(String(e))); }}>Reload</button></div>}
      {notice && <p role="status">{notice}</p>}
      {error && <p className="error" role="alert">{error}</p>}
      {watchError && <p className="warning" role="status">Library refresh: {watchError}. Reload is still available.</p>}
      {catalog?.errors.map((message) => <p className="warning" key={message}>{message}</p>)}
      {change ? <section className="pack-change-review">
        <h3>{change.kind === "remove" ? `Remove ${change.pack.name}?` : `${change.review.current ? "Replace" : "Restore"} ${change.review.replacement.name}`}</h3>
        {change.kind === "replace" && <p>{change.review.current ? `${change.review.current.version} (${change.review.current.icons} icons) → ` : ""}{change.review.replacement.version} ({change.review.replacement.icons} icons)</p>}
        {reviewStale && <p className="warning" role="status">This pack changed while you reviewed it. Cancel and review the change again.</p>}
        <p>Saved artwork and icons already selected in drafts are kept. The previous pack remains available under Archived versions.</p>
        <div className="row"><button type="button" className="primary" disabled={busy || reviewStale} onClick={() => void applyChange()}>{change.kind === "remove" ? "Remove pack" : "Apply replacement"}</button>
          <button type="button" disabled={busy} onClick={() => { setChange(null); setError(""); }}>Cancel change</button></div>
      </section> : <>
      <Check label="Show hidden packs" checked={showHidden} onChange={setShowHidden} />
      <div className="icon-library-filters">
        <Field label="Icon pack"><select value={pack} disabled={busy} onChange={(e) => setPack(e.target.value)}>
          <option value="">All packs</option>{available.map((p) => <option value={p.id} key={p.id}>{p.name} ({p.icons.length}){p.hidden ? " · hidden" : ""}</option>)}
        </select></Field>
        <Field label="Search icons"><input value={query} disabled={busy} onChange={(e) => setQuery(e.target.value)} placeholder="Name, tag or filename" /></Field>
      </div>
      {catalog?.packs.filter((p) => p.id === pack).map((p) => <p className="hint" key={p.id}>{p.author} · {p.version}{p.warnings.length ? ` · ${p.warnings.join("; ")}` : ""}</p>)}
      {selected && <div className="row pack-management">
        <button type="button" disabled={busy || !selected.can_hide} onClick={() => void visibility()}>{selected.hidden ? "Show pack" : "Hide pack"}</button>
        <button type="button" disabled={busy} onClick={() => void replace()}>Update pack…</button>
        <button type="button" disabled={busy || selected.pinned} title={selected.pinned ? "Pinned packs can be hidden, but not removed" : undefined} onClick={() => { setError(""); setChange({ kind: "remove", pack: selected }); }}>Remove pack…</button>
      </div>}
      {!catalog && !error && <p role="status">Loading icon packs…</p>}
      {catalog && matches.length === 0 && <p role="status">{catalog.packs.length ? (catalog.packs.every((p) => p.hidden) && !showHidden && !search ? "Packs are hidden. Show hidden packs or search to find their icons." : "No icons match your search.") : "Import a .streamDeckIconPack file or .sdIconPack folder to start."}</p>}
      <div className="pack-icon-grid">
        {matches.slice(0, limit).map(({ pack: p, icon }) => <button type="button" disabled={busy || !onChoose} key={`${p.id}/${p.revision}/${icon.path}`}
          aria-label={icon.name} title={`${p.name} · ${icon.path}`} onClick={() => void choose(p, icon)}>
          <Thumbnail id={p.id} revision={p.revision} icon={icon} /><span>{icon.name}</span>
        </button>)}
      </div>
      {matches.length > limit && <button type="button" disabled={busy} onClick={() => setLimit((n) => n + 60)}>Show more icons ({matches.length - limit} remaining)</button>}
      {!!catalog?.archives?.length && <details className="pack-archives"><summary>Archived versions ({catalog.archives.length})</summary>
        <ul>{catalog.archives.slice(0, archiveLimit).map((a, index) => <li key={a.token}><span className="pack-archive-info" id={`${archiveDescription}-${index}`}><span>{a.name} · {a.version} · {a.icons} icons</span><small>{a.kind === "remove" ? "Removed" : "Replaced"} · {new Date(a.created * 1000).toLocaleString()}</small></span>
          <button type="button" aria-describedby={`${archiveDescription}-${index}`} disabled={busy} onClick={() => void replace(a)} aria-label={`Restore ${a.name} ${a.version}`}>Restore…</button></li>)}</ul>
        {catalog.archives.length > archiveLimit && <button type="button" disabled={busy} onClick={() => setArchiveLimit((n) => n + 20)}>Show older versions</button>}
      </details>}
      </>}
      <div className="modal-actions"><button type="button" disabled={busy} onClick={onClose}>Cancel</button></div>
    </div>
  </Modal>;
}
