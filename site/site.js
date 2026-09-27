// A working SlopBar inside the page. The frames run the app's real UI (src/ via
// design/demo/app.html and settings.html) against a stand-in backend (design/demo/backend.js);
// this file plays the part of Windows: it places the taskbar widget and its flyout on the mock
// screen, shows the widget's context menu and hosts the Settings window.

const W = 680, H = 500, BAR = 40, WORK_H = H - BAR, MARGIN = 12, FLYOUT_W = 380;
const { state, hooks, snapshot, emit } = window.slopbar;

const $ = (id) => document.getElementById(id);
const stage = $("stage"), screen = $("screen"), menu = $("menu");
const frames = { dock: $("dock"), panel: $("panel"), accounts: $("accounts-frame") };
const flyout = $("flyout");
let panelHeight = 320;

// ---------------------------------------------------------------------------------------------
// Taskbar theme: the page's color scheme to start with, then whatever the toggle picks. The app
// polls taskbar_theme; set it on the open frames too so the switch is immediate.

const dark = matchMedia("(prefers-color-scheme: dark)");
function setTheme(theme) {
  state.theme = theme;
  document.documentElement.dataset.taskbar = theme;
  for (const frame of Object.values(frames)) {
    const doc = frame.contentDocument;
    if (doc?.documentElement) doc.documentElement.dataset.theme = theme;
  }
  $("theme-toggle").textContent = theme === "dark" ? "light" : "dark";
}
setTheme(dark.matches ? "dark" : "light");
$("theme-toggle").addEventListener("click", () => setTheme(state.theme === "dark" ? "light" : "dark"));

// ---------------------------------------------------------------------------------------------
// The widget and its flyout

hooks.set_dock_width = ({ width }) => {
  // A widget that hasn't rendered yet measures ~0; keep the last real width.
  if (width > 20) frames.dock.style.width = `${width}px`;
  placeFlyout();
};
hooks.set_panel_height = ({ height }, label) => {
  if (label === "accounts") {
    frames.accounts.style.height = `${height}px`;
    return;
  }
  panelHeight = height;
  placeFlyout();
};

/**
 * Above the widget, centered on it and kept 12 px from the taskbar and the screen edge. The mock
 * screen is shorter than a real one, so a flyout taller than it scrolls.
 */
function placeFlyout() {
  const h = Math.min(panelHeight, WORK_H - MARGIN * 2);
  const center = frames.dock.offsetLeft + frames.dock.offsetWidth / 2;
  const x = Math.min(Math.max(center - FLYOUT_W / 2, MARGIN), W - FLYOUT_W - MARGIN);
  Object.assign(flyout.style, { left: `${x}px`, top: `${WORK_H - h - MARGIN}px`, height: `${h}px` });
  frames.panel.style.height = `${panelHeight}px`;
}

function setPanel(open) {
  if (open === flyout.classList.contains("open")) return;
  flyout.classList.toggle("open", open);
  // The app replays the flyout's entrance when its window gets focus.
  if (open) frames.panel.contentWindow.focus();
  emit("panel-visible", open);
}

hooks.dock_clicked = () => {
  stage.classList.add("touched");
  closeMenu();
  setPanel(!flyout.classList.contains("open"));
};

// Right click: the native context menu, drawn here with the labels the app sent.
hooks.dock_menu = () => {
  stage.classList.add("touched");
  setPanel(false);
  for (const b of menu.querySelectorAll("button")) b.textContent = state.menuLabels[b.dataset.action];
  menu.hidden = false;
  const center = frames.dock.offsetLeft + frames.dock.offsetWidth / 2;
  menu.style.left = `${Math.min(center - menu.offsetWidth / 2, W - menu.offsetWidth - 4)}px`;
  menu.style.top = `${WORK_H - menu.offsetHeight - 4}px`;
};
function closeMenu() {
  menu.hidden = true;
}
menu.addEventListener("click", (e) => {
  const action = e.target.closest("button")?.dataset.action;
  if (!action) return;
  e.stopPropagation();
  closeMenu();
  if (action === "refresh") emit("usage-updated", snapshot());
  if (action === "settings") openSettings();
  if (action === "quit") quit();
});

// Quitting takes the widget off the taskbar; the desktop shortcut starts it again.
function quit() {
  setPanel(false);
  frames.dock.hidden = true;
  $("shortcut").hidden = false;
}
$("shortcut").addEventListener("click", (e) => {
  e.stopPropagation();
  $("shortcut").hidden = true;
  frames.dock.hidden = false;
  placeFlyout();
});

// A click anywhere else takes focus away, which closes the flyout and the menu.
addEventListener("click", (e) => {
  if (e.target.closest("#menu")) return;
  closeMenu();
  setPanel(false);
});
addEventListener("contextmenu", (e) => {
  if (e.target.closest("#screen")) e.preventDefault();
  closeMenu();
});
addEventListener("blur", closeMenu);

// ---------------------------------------------------------------------------------------------
// Settings window, as a modal over the page.

const modal = $("settings-modal");
function openSettings() {
  setPanel(false);
  modal.hidden = false;
  document.body.style.overflow = "hidden";
  fitSettings();
}
hooks.open_settings = openSettings;
function closeSettings() {
  modal.hidden = true;
  document.body.style.overflow = "";
}
function fitSettings() {
  const s = Math.min(1, (innerWidth - 32) / 820, (innerHeight - 96) / 600);
  modal.style.setProperty("--s", s);
}
$("settings-close").addEventListener("click", closeSettings);
modal.addEventListener("click", (e) => { if (e.target === modal) closeSettings(); });
addEventListener("keydown", (e) => { if (e.key === "Escape" && !modal.hidden) closeSettings(); });

// ---------------------------------------------------------------------------------------------
// Fit the mock screen to the column.

function fit() {
  stage.style.setProperty("--s", Math.min(1, stage.clientWidth / W));
  fitSettings();
}
fit();
addEventListener("resize", fit);
placeFlyout();
