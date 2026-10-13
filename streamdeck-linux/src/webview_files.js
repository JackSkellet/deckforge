// Inspector-only adapter for the native file chooser. No filesystem API is
// exposed: paths enter this map only after the native chooser accepts files.
(() => {
  if (location.protocol + '//' + location.host !== __ORIGIN__) return;
  const root = window.top;
  try { if (root.location.protocol + '//' + root.location.host !== location.protocol + '//' + location.host) return; } catch { return; }
  const key = '__deckforgeFileChooser';
  if (!root[key]) {
    const values = new WeakMap(), requests = new Map();
    let pending;
    Object.defineProperty(root, key, {value: {
      track(input) { if (input.type === 'file' && !input.disabled) pending = input; },
      begin() {
        if (!pending?.isConnected || pending.type !== 'file') return null;
        const id = crypto.randomUUID(); requests.set(id, pending); pending = null; return id;
      },
      complete(id, paths) {
        const input = requests.get(id); requests.delete(id);
        if (!input?.isConnected || input.type !== 'file') return false;
        if (paths !== null) values.set(input, paths);
        return true;
      },
      paths(input) { return values.get(input); },
      clear(input) { values.delete(input); }
    }});
  }
  const adapter = root[key], proto = HTMLInputElement.prototype;
  const value = Object.getOwnPropertyDescriptor(proto, 'value');
  Object.defineProperty(proto, 'value', {...value,
    get() {
      const native = value.get.call(this), paths = adapter.paths(this);
      return this.type === 'file' && native && paths?.length ? encodeURI(paths[0]) : native;
    },
    set(next) { value.set.call(this, next); adapter.clear(this); }
  });
  const files = Object.getOwnPropertyDescriptor(proto, 'files');
  Object.defineProperty(proto, 'files', {...files,
    set(next) { files.set.call(this, next); adapter.clear(this); }
  });
  for (const name of ['click', 'showPicker']) {
    const original = proto[name];
    if (!original) continue;
    proto[name] = function(...args) { adapter.track(this); return original.apply(this, args); };
  }
  document.addEventListener('click', event => {
    const input = event.composedPath().find(node => node instanceof HTMLInputElement && node.type === 'file');
    if (input) adapter.track(input);
  }, true);
  document.addEventListener('reset', event => {
    // A cancelled reset leaves the selection intact.
    queueMicrotask(() => { if (!event.defaultPrevented) for (const input of event.target.elements || []) adapter.clear(input); });
  }, true);
})();
