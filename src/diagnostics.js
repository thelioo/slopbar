// Deliberately never send exception messages, stack traces, URLs or rejected values.
export function observeFrontend(surface) {
  const invoke = window.__TAURI__.core.invoke;
  let sent = 0;
  const report = (kind, line = null, column = null) => {
    if (sent++ >= 20) return; // An error loop must not fill the rolling journal.
    invoke("record_frontend_error", { surface, kind, line, column }).catch(() => {});
  };
  window.addEventListener("error", e => {
    const kinds = new Map([["Error", "error"], ["TypeError", "type_error"], ["ReferenceError", "reference_error"], ["SyntaxError", "syntax_error"], ["RangeError", "range_error"]]);
    const kind = kinds.get(e.error?.name) ?? "other";
    const numeric = n => Number.isInteger(n) && n >= 0 && n <= 4294967295 ? n : null;
    report(kind, numeric(e.lineno), numeric(e.colno));
  });
  window.addEventListener("unhandledrejection", () => report("rejection"));
}
