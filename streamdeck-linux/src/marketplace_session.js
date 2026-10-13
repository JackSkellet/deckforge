// Runs only in Deckforge's isolated Marketplace browser. No editor/plugin bridge.
(() => {
  if (window !== window.top || location.origin !== "https://marketplace.elgato.com" || location.pathname.startsWith("/auth/")) return;
  if (window.__deckforgeMarketplaceBusy) return;
  window.__deckforgeMarketplaceBusy = true;
  const operation = __OPERATION__;
  const variant = __VARIANT__;
  const send = value => window.ipc.postMessage(JSON.stringify({ nonce: __NONCE__, reply: value }));
  const get = async path => {
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(), 25000);
    try {
      const response = await fetch(path, {
        credentials: "include", cache: "no-store", signal: abort.signal,
        headers: { "Content-Type": "application/json" },
      });
      if (!response.ok) return { status: response.status };
      return { status: response.status, data: await response.json() };
    } finally { clearTimeout(timer); }
  };
  (async () => {
    const session = await get("/api/auth/session");
    if (session.status === 401 || (session.status === 200 && !session.data?.user)) {
      send({ kind: "signed_out" }); return;
    }
    if (session.status !== 200 || !session.data?.user) {
      send({ kind: "error", error: "Marketplace session check failed. Try again." }); return;
    }
    const name = typeof session.data.user.name === "string" ? session.data.user.name.slice(0, 120) : "Elgato account";
    if (operation !== "install") { send({ kind: "authenticated", name }); return; }
    const result = await get("/api/proxy/purchase-link/" + variant);
    if (result.status === 401) { send({ kind: "signed_out" }); return; }
    if (result.status === 403 || result.status === 404) {
      send({ kind: "error", name, error: "This account cannot download this plugin. Get it in Marketplace with the same account, then retry." }); return;
    }
    if (result.status !== 200 || typeof result.data?.direct_link !== "string") {
      send({ kind: "error", name, error: "Marketplace did not provide a download link. Try again later." }); return;
    }
    send({ kind: "download", name, url: result.data.direct_link });
  })().catch(() => send({ kind: "error", error: "Could not reach Marketplace. Check your connection and retry." }));
})();
