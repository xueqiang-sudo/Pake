// Dedalix: Mattermost DesktopAPI shim
// Bridges Mattermost's desktop-specific API calls to Pake's Tauri commands.
// Mattermost detects window.desktopAPI and uses it for badge counts, theme,
// and other desktop integrations. Without this shim, those features are silent.
(function () {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return;

  // Prevent double-init if the script runs more than once
  if (window.desktopAPI) return;

  window.desktopAPI = {
    // Badge / unread count bridge
    // Called by Mattermost's UnreadsStatusHandler whenever mention/unread state changes.
    setUnreadsAndMentions: (isUnread, mentionCount) => {
      if (mentionCount > 0) {
        invoke("set_dock_badge", { count: mentionCount }).catch(() => {});
      } else if (isUnread) {
        invoke("set_dock_badge_label", { label: "•" }).catch(() => {});
      } else {
        invoke("clear_dock_badge").catch(() => {});
      }
    },

    // Theme sync: Mattermost queries this on init
    getDarkMode: () => {
      if (
        window.matchMedia &&
        window.matchMedia("(prefers-color-scheme: dark)").matches
      ) {
        return Promise.resolve(true);
      }
      return Promise.resolve(false);
    },

    // App lifecycle stubs (called by Mattermost on init/state changes)
    reactAppInitialized: () => {},
    setSessionExpired: () => {},
    updateTheme: () => {},

    // App info: Mattermost uses this to detect desktop app version
    getAppInfo: () => Promise.resolve({ name: "Dedalix", version: "1.0.0" }),

    // Dev mode check
    isDev: () => Promise.resolve(false),

    // Popout support stub (Mattermost may query this)
    canPopout: () => Promise.resolve(false),
  };
})();
