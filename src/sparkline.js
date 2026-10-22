// A small inline sparkline of an account's weekly usage over the last 7 days, from the readings
// SlopBar recorded (get_sparklines). Plain SVG colored by the page's theme tokens; the numbers it
// shows are in its text alternative, so it never relies on color or shape alone.
import { t } from "./i18n.js";

const DAY = 86400000;
const SPAN = 7 * DAY;
/** Readings further apart than this are a gap in the line, not a slope. */
const GAP = 3 * 3600000;

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);

/** What the sparkline shows, in words: "Weekly usage, last 7 days: 12% to 48%, peak 61%". */
export function sparklineText(points) {
  if (!points?.length) return t("sparkNone");
  const values = points.map(([, v]) => Math.round(v));
  return t("sparkText", { from: values[0], to: values.at(-1), peak: Math.max(...values) });
}

/**
 * The sparkline's SVG for `points` ([milliseconds, percent] pairs, oldest first). The x axis is
 * the last 7 days ending now and the y axis 0–100%, so every account's line reads the same way.
 * Without readings it keeps its place, empty, so rows line up.
 */
export function sparkline(points, { width = 56, height = 16, now = Date.now() } = {}) {
  const label = sparklineText(points);
  const pad = 1.5;
  const x = (at) => pad + ((Math.min(now, Math.max(now - SPAN, at)) - (now - SPAN)) / SPAN) * (width - 2 * pad);
  const y = (v) => height - pad - (Math.min(100, Math.max(0, v)) / 100) * (height - 2 * pad);
  let d = "";
  let prev = null;
  for (const [at, v] of points ?? []) {
    if (at < now - SPAN) continue;
    d += `${prev == null || at - prev > GAP ? "M" : "L"}${x(at).toFixed(1)} ${y(v).toFixed(1)}`;
    prev = at;
  }
  const last = points?.at(-1);
  const tone = !last ? "" : last[1] >= 90 ? "bad" : last[1] >= 70 ? "warn" : "ok";
  return `<svg class="spark" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" role="img" aria-label="${esc(label)}">`
    + `<title>${esc(label)}</title>`
    + `<path class="spark-base" d="M${pad} ${height - pad}H${width - pad}"/>`
    + (d ? `<path class="spark-line" d="${d}"/>` : "")
    + (last && last[0] >= now - SPAN ? `<circle class="spark-dot ${tone}" cx="${x(last[0]).toFixed(1)}" cy="${y(last[1]).toFixed(1)}" r="1.8"/>` : "")
    + `</svg>`;
}
