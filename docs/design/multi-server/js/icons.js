// Perfuse icon set for the multi-server prototype.
// Same contract as assets/icons: 24×24 viewBox, 1.5 stroke, round caps/joins, currentColor.
// `file` = target filename in assets/icons; `isNew` marks glyphs this design adds.
window.PERFUSE_ICONS = {
  // ── existing (copied verbatim from assets/icons) ──
  home: { file: 'home.svg', label: 'Home', body: '<path d="M4 10.5L12 4l8 6.5V19a1 1 0 0 1-1 1h-4.5v-6h-5v6H5a1 1 0 0 1-1-1z"/>' },
  movies: { file: 'movies.svg', label: 'Movies', body: '<rect x="4" y="4" width="6.5" height="6.5" rx="1.5"/><rect x="13.5" y="4" width="6.5" height="6.5" rx="1.5"/><rect x="4" y="13.5" width="6.5" height="6.5" rx="1.5"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="1.5"/>' },
  'tv-shows': { file: 'tv-shows.svg', label: 'TV shows', body: '<rect x="3" y="5" width="18" height="12" rx="2"/><path d="M8 20h8M12 17v3"/>' },
  settings: { file: 'settings.svg', label: 'Servers & settings', body: '<path d="M18.69 9.93L21.07 10.44L21.07 13.56L18.69 14.07L18.19 15.27L19.51 17.31L17.31 19.51L15.27 18.19L14.07 18.69L13.56 21.07L10.44 21.07L9.93 18.69L8.73 18.19L6.69 19.51L4.49 17.31L5.81 15.27L5.31 14.07L2.93 13.56L2.93 10.44L5.31 9.93L5.81 8.73L4.49 6.69L6.69 4.49L8.73 5.81L9.93 5.31L10.44 2.93L13.56 2.93L14.07 5.31L15.27 5.81L17.31 4.49L19.51 6.69L18.19 8.73z"/><circle cx="12" cy="12" r="3"/>' },
  sort: { file: 'sort.svg', label: 'Sort libraries', body: '<path d="M4 7h11M4 12h7M4 17h4M17 7v11M14 15l3 3 3-3"/>' },
  server: { file: 'server.svg', label: 'Server', body: '<rect x="4" y="4" width="16" height="7" rx="1.5"/><rect x="4" y="13" width="16" height="7" rx="1.5"/><path d="M7.5 7.5h.01M7.5 16.5h.01"/>' },
  play: { file: 'play.svg', label: 'Play', body: '<path d="M8 5.6v12.8a1 1 0 0 0 1.5.86l10.2-6.4a1 1 0 0 0 0-1.72L9.5 4.74A1 1 0 0 0 8 5.6z" fill="currentColor" stroke="none"/>' },
  pause: { file: 'pause.svg', label: 'Pause', body: '<rect x="6.5" y="5" width="4" height="14" rx="1.2" fill="currentColor" stroke="none"/><rect x="13.5" y="5" width="4" height="14" rx="1.2" fill="currentColor" stroke="none"/>' },
  'chapter-next': { file: 'chapter-next.svg', label: 'Next chapter', body: '<path d="M18 5.5v13"/><path d="M6 6.6v10.8a1 1 0 0 0 1.56.83l7.24-5.4a1 1 0 0 0 0-1.66L7.56 5.77A1 1 0 0 0 6 6.6z"/>' },
  'chapter-previous': { file: 'chapter-previous.svg', label: 'Previous chapter', body: '<path d="M6 5.5v13"/><path d="M18 6.6v10.8a1 1 0 0 1-1.56.83L9.2 12.83a1 1 0 0 1 0-1.66l7.24-5.4A1 1 0 0 1 18 6.6z"/>' },
  speed: { file: 'speed.svg', label: 'Speed', body: '<path d="M4.6 17.5a8 8 0 1 1 14.8 0"/><path d="M12 14l3.8-3.8"/><circle cx="12" cy="14" r="1.2" fill="currentColor" stroke="none"/>' },
  back: { file: 'back.svg', label: 'Back', body: '<path d="M15 5l-7 7 7 7"/>' },
  refresh: { file: 'refresh.svg', label: 'Refresh', body: '<path d="M20 12a8 8 0 1 1-8-8c2.2 0 4.2.9 5.7 2.3L20 8.5"/><path d="M20 4v4.5h-4.5"/>' },
  close: { file: 'close.svg', label: 'Close', body: '<path d="M6 6l12 12M18 6L6 18"/>' },
  alert: { file: 'alert.svg', label: 'Error', body: '<path d="M10.7 4.8a1.5 1.5 0 0 1 2.6 0l7.4 12.9a1.5 1.5 0 0 1-1.3 2.3H4.6a1.5 1.5 0 0 1-1.3-2.3z"/><path d="M12 10v3.5M12 16.8h.01"/>' },
  'sign-in-browser': { file: 'sign-in-browser.svg', label: 'Sign in with Plex', body: '<path d="M13 4h7v7M20 4l-9 9"/><path d="M18 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10"/>' },
  'play-from-beginning': { file: 'play-from-beginning.svg', label: 'Play from beginning', body: '<path d="M4 12a8 8 0 1 0 8-8c-2.2 0-4.2.9-5.7 2.3L4 8.5"/><path d="M4 4v4.5h4.5"/><path d="M10.5 9.5v5l4-2.5z" fill="currentColor" stroke="none"/>' },
  resume: { file: 'resume.svg', label: 'Resume', body: '<path d="M12 3a9 9 0 1 1-8.5 6"/><path d="M10.2 9v6l4.8-3z" fill="currentColor" stroke="none"/>' },
  forget: { file: 'forget.svg', label: 'Remove', body: '<path d="M4.5 7h15M9.5 7V5a1 1 0 0 1 1-1h3a1 1 0 0 1 1 1v2M6.5 7l.8 11.6a1.5 1.5 0 0 0 1.5 1.4h6.4a1.5 1.5 0 0 0 1.5-1.4L17.5 7"/>' },
  'sign-out': { file: 'sign-out.svg', label: 'Sign out', body: '<path d="M10 4H6.5A1.5 1.5 0 0 0 5 5.5v13A1.5 1.5 0 0 0 6.5 20H10M15 8l4 4-4 4M19 12h-9"/>' },
  'volume-high': { file: 'volume-high.svg', label: 'Volume', body: '<path d="M4 9.5h3l4.5-4v13L7 14.5H4z"/><path d="M15 9a4 4 0 0 1 0 6M17.5 6.5a7.5 7.5 0 0 1 0 11"/>' },
  'page-next': { file: 'page-next.svg', label: 'Next page', body: '<path d="M9.5 6l6 6-6 6"/>' },
  'page-previous': { file: 'page-previous.svg', label: 'Previous page', body: '<path d="M14.5 6l-6 6 6 6"/>' },
  'status-connected': { file: 'status-connected.svg', label: 'Connected', body: '<circle cx="12" cy="12" r="4" fill="currentColor" stroke="none"/>' },
  pin: { file: 'pin.svg', label: 'Always on top', body: '<path d="M12 17v5M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>' },
  minimize: { file: 'minimize.svg', label: 'Minimize', body: '<path d="M6 12h12"/>' },
  maximize: { file: 'maximize.svg', label: 'Maximize', body: '<rect x="6" y="6" width="12" height="12" rx="1.5"/>' },

  // ── new: server types (monochrome glyphs, simplified to the stroke family) ──
  'server-plex': { file: 'server-plex.svg', label: 'Plex', isNew: true, use: 'Server-type mark: sidebar group header, server cards, settings rows, status popover', body: '<path d="M6.5 4h5l6 8-6 8h-5l6-8z"/>' },
  'server-jellyfin': { file: 'server-jellyfin.svg', label: 'Jellyfin', isNew: true, use: 'Server-type mark: sidebar group header, server cards, settings rows, status popover', body: '<path d="M10.6 5.3a1.6 1.6 0 0 1 2.8 0l6.5 11.5a1.6 1.6 0 0 1-1.4 2.4H5.5a1.6 1.6 0 0 1-1.4-2.4z"/><path d="M12 10.5l-2.8 5h5.6z"/>' },
  'server-audiobookshelf': { file: 'server-audiobookshelf.svg', label: 'Audiobookshelf', isNew: true, use: 'Server-type mark: sidebar group header, server cards, settings rows, status popover', body: '<path d="M4.5 13.5V12a7.5 7.5 0 0 1 15 0v1.5"/><rect x="3.5" y="13.5" width="3.5" height="6" rx="1.2"/><rect x="17" y="13.5" width="3.5" height="6" rx="1.2"/><path d="M10 12.5v7M12.5 12.5v7M14.3 13l1.2 6.3"/>' },

  // ── new: content types ──
  audiobooks: { file: 'audiobooks.svg', label: 'Audiobooks', isNew: true, use: 'Sidebar library icon for audiobook libraries', body: '<path d="M12 6.5C10.2 5.2 7.7 4.5 4.5 4.5v13c3.2 0 5.7.7 7.5 2 1.8-1.3 4.3-2 7.5-2v-13c-3.2 0-5.7.7-7.5 2z"/><path d="M12 6.5v13"/>' },
  podcasts: { file: 'podcasts.svg', label: 'Podcasts', isNew: true, use: 'Sidebar library icon for podcast libraries', body: '<circle cx="12" cy="10" r="2.5"/><path d="M12 14.5v5.5"/><path d="M7.4 14.6a6.5 6.5 0 1 1 9.2 0"/>' },
  headphones: { file: 'headphones.svg', label: 'Listening', isNew: true, use: 'Content-type badge on Continue Listening cards; docked player expand target', body: '<path d="M4 15v-3a8 8 0 0 1 16 0v3"/><rect x="4" y="14" width="3.5" height="6" rx="1.2"/><rect x="16.5" y="14" width="3.5" height="6" rx="1.2"/>' },

  // ── new: audiobook playback ──
  'back-30': { file: 'back-30.svg', label: 'Back 30s', isNew: true, use: 'Audiobook player skip back', body: '<path d="M12 4.5a7.5 7.5 0 1 1-7.5 7.5"/><path d="M14.2 2.3L12 4.5l2.2 2.2"/><path d="M7.6 9.2h2.3L8.7 11.4c.9 0 1.5.6 1.5 1.5a1.6 1.6 0 0 1-2.8 1.1"/><rect x="12" y="9" width="3.4" height="6" rx="1.7"/>' },
  'forward-30': { file: 'forward-30.svg', label: 'Forward 30s', isNew: true, use: 'Audiobook player skip forward', body: '<path d="M12 4.5a7.5 7.5 0 1 0 7.5 7.5"/><path d="M9.8 2.3L12 4.5 9.8 6.7"/><path d="M7.6 9.2h2.3L8.7 11.4c.9 0 1.5.6 1.5 1.5a1.6 1.6 0 0 1-2.8 1.1"/><rect x="12" y="9" width="3.4" height="6" rx="1.7"/>' },
  chapters: { file: 'chapters.svg', label: 'Chapters', isNew: true, use: 'Open chapter list (player, docked bar)', body: '<path d="M4 6h2.5M4 12h2.5M4 18h2.5M10 6h10M10 12h10M10 18h6"/>' },
  'sleep-timer': { file: 'sleep-timer.svg', label: 'Sleep timer', isNew: true, use: 'Sleep timer menu button', body: '<path d="M18.5 14.2A7 7 0 1 1 9.8 5.5a5.6 5.6 0 0 0 8.7 8.7z"/><path d="M15 4.5h3l-3 3.5h3"/>' },
  bookmark: { file: 'bookmark.svg', label: 'Bookmark', isNew: true, use: 'Bookmark list rows', body: '<path d="M7 4.5h10a1 1 0 0 1 1 1V20l-6-4-6 4V5.5a1 1 0 0 1 1-1z"/>' },
  'bookmark-add': { file: 'bookmark-add.svg', label: 'Add bookmark', isNew: true, use: 'Add bookmark at the current position', body: '<path d="M7 4.5h10a1 1 0 0 1 1 1V20l-6-4-6 4V5.5a1 1 0 0 1 1-1z"/><path d="M12 8v5M9.5 10.5h5"/>' },
  'player-expand': { file: 'player-expand.svg', label: 'Open player', isNew: true, use: 'Docked bar → full player', body: '<path d="M6 14.5l6-6 6 6"/>' },
  'player-collapse': { file: 'player-collapse.svg', label: 'Minimise player', isNew: true, use: 'Full player → docked bar', body: '<path d="M6 9.5l6 6 6-6"/>' },
  'now-playing': { file: 'now-playing.svg', label: 'Now playing', isNew: true, use: 'Current chapter marker in chapter lists', body: '<path d="M6 10v4M10 7v10M14 9v6M18 11v2"/>' },

  // ── new: server management ──
  add: { file: 'add.svg', label: 'Add server', isNew: true, use: 'Add server buttons', body: '<path d="M12 5v14M5 12h14"/>' },
  check: { file: 'check.svg', label: 'Done', isNew: true, use: 'Completed chapter, server added, selected menu item', body: '<path d="M5 12.5l4.5 4.5L19 7.5"/>' },
  'chevron-down': { file: 'chevron-down.svg', label: 'Expand', isNew: true, use: 'Sidebar server-group collapse, menus', body: '<path d="M7 10l5 5 5-5"/>' },
  'drag-handle': { file: 'drag-handle.svg', label: 'Reorder', isNew: true, use: 'Settings server row reorder handle', body: '<circle cx="9" cy="6.5" r="1.2" fill="currentColor" stroke="none"/><circle cx="15" cy="6.5" r="1.2" fill="currentColor" stroke="none"/><circle cx="9" cy="12" r="1.2" fill="currentColor" stroke="none"/><circle cx="15" cy="12" r="1.2" fill="currentColor" stroke="none"/><circle cx="9" cy="17.5" r="1.2" fill="currentColor" stroke="none"/><circle cx="15" cy="17.5" r="1.2" fill="currentColor" stroke="none"/>' },
  rename: { file: 'rename.svg', label: 'Rename', isNew: true, use: 'Rename server', body: '<path d="M4.5 19.5h4l10-10a2.8 2.8 0 0 0-4-4l-10 10z"/><path d="M13 7l4 4"/>' },
  'quick-connect': { file: 'quick-connect.svg', label: 'Quick Connect', isNew: true, use: 'Jellyfin Quick Connect sign-in', body: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>' },
  'status-partial': { file: 'status-partial.svg', label: 'Some servers unreachable', isNew: true, use: 'Status footer / server dot when degraded (yellow)', body: '<path d="M12 7.2l4.6 8.1H7.4z" fill="currentColor" stroke="none"/>' },
  'status-offline': { file: 'status-offline.svg', label: 'Not connected', isNew: true, use: 'Status footer / server dot when offline (red)', body: '<circle cx="12" cy="12" r="4.2"/><path d="M9.2 14.8l5.6-5.6"/>' },
  more: { file: 'more.svg', label: 'More actions', isNew: true, use: 'Overflow menu on server rows', body: '<circle cx="12" cy="6" r="1.4" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1.4" fill="currentColor" stroke="none"/><circle cx="12" cy="18" r="1.4" fill="currentColor" stroke="none"/>' },
};

// Inline producer used by the prototype. Always emits .od-icon so size comes from --od-icon-size.
window.icon = function icon(name, opts = {}) {
  const def = window.PERFUSE_ICONS[name];
  if (!def) return '';
  const label = opts.label === undefined ? null : opts.label;
  const a11y = label ? `role="img" aria-label="${label}"` : 'aria-hidden="true" focusable="false"';
  const cls = opts.cls ? ` ${opts.cls}` : '';
  return `<svg class="od-icon${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" ${a11y}>${def.body}</svg>`;
};
