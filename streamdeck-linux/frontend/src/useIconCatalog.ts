import { useCallback, useEffect, useRef, useState } from "react";
import { request } from "./bridge";

/** Refresh only on library notifications. Late reads cannot replace a newer
 * catalog, and automatic reads wait for active imports/reviews to finish. */
export function useIconCatalog<T>(busy: boolean) {
  const [catalog, setCatalog] = useState<T | null>(null);
  const [observerError, setObserverError] = useState("");
  const [loadError, setLoadError] = useState("");
  const alive = useRef(true), serial = useRef(0), inFlight = useRef(0);
  const revision = useRef<number | undefined>(undefined), pending = useRef(true);
  const busyRef = useRef(busy); busyRef.current = busy;
  const schedule = useRef<() => void>(() => {});
  const refresh = useCallback(async (automatic = false) => {
    const sequence = ++serial.current, observed = revision.current;
    inFlight.current++;
    try {
      const next = await request<T>("icon_packs", { operation: "list" });
      if (alive.current && sequence === serial.current) {
        setCatalog(next); setLoadError("");
        if (observed === revision.current) pending.current = false;
      }
      return next;
    } catch (error) {
      if (automatic && alive.current && sequence === serial.current) setLoadError(String(error));
      throw error;
    } finally {
      inFlight.current--;
      if (alive.current && inFlight.current === 0 && pending.current) schedule.current();
    }
  }, []);
  useEffect(() => {
    alive.current = true; pending.current = true;
    let timer: ReturnType<typeof setTimeout> | undefined, attempts = 0;
    const queue = (delay = 100) => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => { timer = undefined; void update(); }, delay);
    };
    const update = async () => {
      if (!alive.current || !pending.current || inFlight.current > 0) return;
      if (busyRef.current) { queue(200); return; }
      const sequence = serial.current + 1;
      try { await refresh(true); attempts = 0; }
      catch {
        if (alive.current && sequence === serial.current) queue(Math.min(500 * 2 ** Math.min(attempts++, 4), 5000));
      }
    };
    schedule.current = () => queue();
    const stop = window.deckforge?.onSnapshot((snapshot) => {
      const observed = snapshot.icon_library;
      if (!observed) return;
      setObserverError(observed.error || "");
      if (revision.current !== undefined && revision.current !== observed.revision) {
        pending.current = true; queue();
      }
      revision.current = observed.revision;
    });
    void update();
    return () => {
      alive.current = false; serial.current++; schedule.current = () => {};
      if (timer) clearTimeout(timer); stop?.();
    };
  }, [refresh]);
  return { catalog, refresh, watchError: observerError || loadError };
}
