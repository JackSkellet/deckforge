import { useEffect, useId, useRef, useState } from "react";
import { request } from "./bridge";

export interface Application {
  id?: string;
  window_class?: string | null;
  name: string;
  desktop: string;
  executable: string;
}

export function ApplicationIcon({ desktop, name }: { desktop: string; name: string }) {
  const element = useRef<HTMLSpanElement>(null);
  const [image, setImage] = useState<{ desktop: string; src: string } | null>(null);
  useEffect(() => {
    let live = true;
    const load = () => {
      void request<string | null>("application_icon", { desktop })
        .then((src) => {
          if (live && src?.startsWith("data:image/png;base64,")) setImage({ desktop, src });
        })
        .catch(() => {}); // Missing/unreadable artwork keeps the labelled fallback.
    };
    let observer: IntersectionObserver | undefined;
    if (typeof IntersectionObserver === "undefined") load();
    else {
      observer = new IntersectionObserver((entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          observer?.disconnect();
          load();
        }
      });
      if (element.current) observer.observe(element.current);
    }
    return () => { live = false; observer?.disconnect(); };
  }, [desktop]);
  return <span className="application-icon" ref={element} aria-hidden="true">
    {image?.desktop === desktop
      ? <img src={image.src} alt="" onError={() => setImage(null)} />
      : <span>{[...name.trim()][0]?.toUpperCase() || "?"}</span>}
  </span>;
}

export function ApplicationPicker({ applications, search, value, onSelect }: {
  applications: Application[];
  search: string;
  value: string;
  onSelect: (app: Application) => void;
}) {
  const group = useId();
  const [limit, setLimit] = useState(40);
  useEffect(() => setLimit(40), [search]);
  const query = search.trim().toLowerCase();
  const filtered = applications.filter((app) =>
    [app.name, app.executable, app.id || "", app.desktop].some((v) => v.toLowerCase().includes(query)),
  );
  return <fieldset className="application-picker">
    <legend>Installed application</legend>
    <div className="application-choices">
      {filtered.slice(0, limit).map((app, index) => <label className="application-choice" key={app.desktop}>
        <input type="radio" name={group} value={app.desktop} checked={value === app.desktop}
          aria-label={app.name} aria-describedby={`${group}-${index}`}
          onChange={() => onSelect(app)} />
        <ApplicationIcon desktop={app.desktop} name={app.name} />
        <span className="application-name"><span>{app.name}</span>
          <small id={`${group}-${index}`} title={app.desktop}>{app.id || app.desktop}</small>
        </span>
      </label>)}
      {filtered.length === 0 && <p className="hint" role="status">No matching installed applications.</p>}
    </div>
    {filtered.length > limit && <button type="button" onClick={() => setLimit((n) => n + 40)}>
      Show more applications ({filtered.length - limit} remaining)
    </button>}
  </fieldset>;
}
