// Stand-in for the app's Rust backend, for pages that run the real UI in frames (the website
// and the GIF recordings). The frames (app.html, settings.html) connect to it through
// window.slopbar; it answers the same commands, keeps settings and demo accounts, and emits the
// same events. Window-level commands (opening the flyout, the context menu, Settings, sizing)
// go to the hosting page through `hooks`.

window.slopbar = (() => {
  const hours = (h) => new Date(Date.now() + h * 3.6e6).toISOString();
  const state = {
    settings: {
      refresh_minutes: 5, providers: { claude: true, codex: true }, language: "auto",
      launch_at_login: false, auto_update: true, aliases: { "claude:work": "Work", "claude:personal": "Personal" },
      balancer: { claude: { auto: false, threshold: 90 }, codex: { auto: false, threshold: 90 } },
    },
    accounts: [
      { id: "work", provider: "claude", email: "you@acme.com", plan: "max", org: "Acme", active: true,
        windows: [{ kind: "session", used_percent: 64, resets_at: hours(2.4) }, { kind: "weekly", used_percent: 38, resets_at: hours(70) }], resets: [] },
      { id: "personal", provider: "claude", email: "you@gmail.com", plan: "pro", org: null, active: false,
        windows: [{ kind: "session", used_percent: 12, resets_at: hours(4.1) }, { kind: "weekly", used_percent: 21, resets_at: hours(96) }], resets: [] },
      { id: "codex", provider: "codex", email: "you@example.com", plan: "pro", org: null, active: true,
        windows: [{ kind: "session", used_percent: 41, resets_at: hours(3) }, { kind: "weekly", used_percent: 76, resets_at: hours(44) }],
        resets: [{ kind: "full", title: "Full reset", count: 1, usable: false, expires_at: hours(24 * 26), next_available_at: null }] },
    ],
    theme: "dark",
    menuLabels: { refresh: "Refresh", settings: "Settings…", quit: "Quit" },
    switchDelay: 450,
  };
  const listeners = {};
  const hooks = {};
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  function snapshot() {
    const alias = (a) => state.settings.aliases[`${a.provider}:${a.id}`] ?? null;
    return {
      accounts: state.accounts
        .filter((a) => state.settings.providers[a.provider])
        .map((a) => ({ sources: a.active ? ["Windows", "WSL: Ubuntu"] : [], credits: null, error: null, stale: false, resets: [],
          ...a, alias: alias(a), fetched_at: new Date().toISOString() })),
      updated_at: new Date().toISOString(),
    };
  }

  function emit(name, payload, only) {
    for (const [label, fns] of Object.entries(listeners)) {
      if (only && only !== label) continue;
      fns[name]?.({ payload });
    }
  }

  async function invoke(label, cmd, args = {}) {
    const s = state.settings;
    switch (cmd) {
      case "get_settings": return structuredClone(s);
      case "get_usage": case "refresh": return snapshot();
      case "system_locale": return navigator.language;
      case "app_version": return "web demo";
      case "taskbar_theme": return state.theme;
      case "set_menu_labels": state.menuLabels = args.labels; return null;
      case "save_settings":
        state.settings = structuredClone(args.settings);
        emit("settings-changed", structuredClone(state.settings));
        emit("usage-updated", snapshot());
        return null;
      case "switch_account": {
        const target = state.accounts.find((a) => a.provider === args.provider && a.id === args.id);
        await sleep(state.switchDelay);
        for (const a of state.accounts) if (a.provider === args.provider) a.active = a === target;
        const snap = snapshot();
        emit("usage-updated", snap);
        emit("account-switched", { provider: args.provider, from: null, to: s.aliases[`${args.provider}:${args.id}`] ?? target.email, auto: false });
        return snap;
      }
      case "remove_account": {
        const a = state.accounts.find((x) => x.provider === args.provider && x.id === args.id);
        if (a?.active) throw "active";
        state.accounts = state.accounts.filter((x) => x !== a);
        const snap = snapshot();
        emit("usage-updated", snap);
        return snap;
      }
      case "add_account": throw "Adding accounts needs the app on your PC — download it to try this.";
      case "check_for_updates": return { current: "web demo", latest: null };
      // set_dock_width, set_panel_height, dock_clicked, dock_menu, open_settings
      default: return hooks[cmd]?.(args, label) ?? null;
    }
  }

  return {
    state, hooks, snapshot, emit,
    connect(win, label) {
      listeners[label] = {};
      return {
        core: { invoke: (cmd, args) => invoke(label, cmd, args) },
        event: { listen: async (name, fn) => { listeners[label][name] = fn; return () => {}; } },
        window: { getCurrentWindow: () => ({ label }) },
      };
    },
  };
})();
