// Stand-in for the Tauri API, for the app's pages running in a browser (scripts/site.mjs builds
// design/demo/app.html and settings.html from src/ with this and backend.js loaded first). The
// page's window label comes from ?label= (dock, panel or settings). Inside a page marked
// data-slopbar-host, the hosting page's backend serves every frame, so they share state; opened
// on its own, a page uses a backend of its own.
(() => {
  const label = new URLSearchParams(location.search).get("label") || "dock";
  const host = window.parent !== window && window.parent.document.documentElement.hasAttribute("data-slopbar-host");
  // The hosting page's backend may not exist yet when this frame runs: queue calls until it does.
  const ready = new Promise((resolve) => {
    const poll = () => (window.parent.slopbar ? resolve(window.parent.slopbar.connect(window, label)) : setTimeout(poll, 20));
    host ? poll() : resolve(window.slopbar.connect(window, label));
  });
  window.__TAURI__ = {
    core: { invoke: (cmd, args) => ready.then((b) => b.core.invoke(cmd, args)) },
    event: { listen: (name, fn) => ready.then((b) => b.event.listen(name, fn)) },
    window: { getCurrentWindow: () => ({ label }) },
  };
})();
