// Perfuse multi-server prototype.
// One state object drives every view; render() rebuilds the sidebar, content and
// dock from it. Playback ticks update only the live time fields (updateLive) so
// focus and scroll are not disturbed every second.
(() => {
  'use strict';

  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const clone = (o) => JSON.parse(JSON.stringify(o));
  const icon = window.icon;
  const KINDS = window.SERVER_KINDS;
  const LIB_ICONS = window.LIBRARY_ICONS;
  const SPEEDS = [0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3];
  const SLEEP_MINUTES = [15, 30, 45, 60];

  const state = {
    servers: clone(window.SEED_SERVERS),
    videos: clone(window.SEED_VIDEOS),
    books: clone(window.SEED_BOOKS),
    view: { name: 'home' },
    history: [],
    status: { open: false, pinned: false },
    sortAlpha: {},
    player: { bookId: null, playing: false, speed: 1, sleep: null, tab: 'chapters' },
    menu: null, // { type: 'speed' | 'sleep', where: 'dock' | 'player', trigger }
    expanded: new Set(),
    renaming: null,
    onboarding: false,
    flow: null,
    flowCtx: null,
    pendingRemove: null,
    dragId: null,
  };

  // ── lookups ──────────────────────────────────────────────
  const serverById = (id) => state.servers.find((s) => s.id === id);
  const bookById = (id) => state.books.find((b) => b.id === id);
  const videoById = (id) => state.videos.find((v) => v.id === id);
  const libOf = (s, id) => s && s.libraries.find((l) => l.id === id);
  const isOnline = (s) => s && s.status === 'online';
  const itemVisible = (item) => {
    const s = serverById(item.serverId);
    const l = libOf(s, item.libraryId);
    return !!(s && l && l.visible);
  };

  // ── formatting ───────────────────────────────────────────
  const pad = (n) => String(n).padStart(2, '0');
  function fmtClock(sec) {
    sec = Math.max(0, Math.floor(sec));
    const h = Math.floor(sec / 3600), m = Math.floor((sec % 3600) / 60), s = sec % 60;
    return h ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
  }
  function fmtDur(sec) {
    const m = Math.max(0, Math.round(sec / 60));
    if (m < 60) return `${m} min`;
    const h = Math.floor(m / 60), r = m % 60;
    return r ? `${h} h ${r} min` : `${h} h`;
  }
  const fmtSpeed = (x) => `${x}×`;

  // ── book helpers ─────────────────────────────────────────
  const bookTotal = (b) => { const c = b.chapters[b.chapters.length - 1]; return c.start + c.duration; };
  function chapterAt(b, pos) {
    for (let i = b.chapters.length - 1; i >= 0; i--) if (pos >= b.chapters[i].start) return i;
    return 0;
  }
  const inProgress = (b) => b.position > 0 && !b.finished;

  // ── status aggregation (sidebar footer) ─────────────────
  function aggregate() {
    const total = state.servers.length;
    const online = state.servers.filter(isOnline).length;
    if (state.servers.some((s) => s.status === 'checking')) {
      return { level: 'checking', text: 'Checking servers…', title: 'Checking servers', summary: `Contacting ${total} ${total === 1 ? 'server' : 'servers'}.` };
    }
    if (total > 0 && online === total) {
      return { level: 'online', text: total === 1 ? '1 server connected' : `${total} servers connected`, title: 'All servers connected', summary: 'Everything is reachable.' };
    }
    if (online === 0) {
      return { level: 'offline', text: 'Cannot connect to servers', title: 'No servers reachable', summary: 'Check your network connection and that your servers are running.' };
    }
    const down = total - online;
    return { level: 'partial', text: `${online} (of ${total}) servers connected`, title: `${down} ${down === 1 ? 'server' : 'servers'} unreachable`, summary: 'Content from unreachable servers is hidden until they reconnect.' };
  }
  const STATUS_ICON = { online: 'status-connected', partial: 'status-partial', offline: 'status-offline', checking: 'refresh' };
  function statusGlyph(level, label) {
    return `<span class="status-glyph st-${level}">${icon(STATUS_ICON[level], label ? { label } : {})}</span>`;
  }
  function serverStateText(s) {
    if (s.status === 'online') return `Connected · ${s.latencyMs} ms`;
    if (s.status === 'checking') return 'Checking…';
    return `Can't reach server · last seen ${s.lastSeen || 'a while ago'}`;
  }
  const serverLevel = (s) => (s.status === 'online' ? 'online' : s.status === 'checking' ? 'checking' : 'offline');

  // ── placeholder artwork (content SVG, same style as the demo fixture) ──
  let artSeq = 0;
  function wrapWords(text, max) {
    const lines = [];
    let cur = '';
    for (const w of text.split(' ')) {
      if ((cur + ' ' + w).trim().length > max && cur) { lines.push(cur); cur = w; } else cur = (cur + ' ' + w).trim();
    }
    if (cur) lines.push(cur);
    return lines;
  }
  function art(palette, kind, opts = {}) {
    const [top, bottom] = window.PALETTES[palette] || window.PALETTES.slate;
    const id = `art${++artSeq}`;
    const [w, h] = { landscape: [232, 130], poster: [168, 252], square: [200, 200] }[kind];
    const sunR = kind === 'poster' ? 26 : kind === 'square' ? 20 : 12;
    const sunX = kind === 'square' ? w * 0.74 : w * 0.68;
    const sunY = kind === 'square' ? h * 0.5 : h * 0.3;
    const hy = kind === 'square' ? h * 0.66 : h * 0.68;
    let text = '';
    if (kind === 'square' && opts.title) {
      const lines = wrapWords(opts.title, 15).slice(0, 3);
      text = lines.map((l, i) => `<text x="16" y="${34 + i * 22}" font-size="18" fill="#ffffff" font-family="Ubuntu Perfuse, Ubuntu, sans-serif" font-weight="300">${esc(l)}</text>`).join('');
      if (opts.author) text += `<text x="16" y="${h - 16}" font-size="10" letter-spacing="1.4" fill="rgba(255,255,255,.82)" font-family="Ubuntu Perfuse, Ubuntu, sans-serif">${esc(opts.author.toUpperCase())}</text>`;
    }
    const a11y = opts.alt ? `role="img" aria-label="${esc(opts.alt)}"` : 'aria-hidden="true"';
    return `<svg class="art-svg" viewBox="0 0 ${w} ${h}" ${a11y}><defs><linearGradient id="${id}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${top}"/><stop offset="1" stop-color="${bottom}"/></linearGradient></defs><rect width="${w}" height="${h}" fill="url(#${id})"/><circle cx="${sunX}" cy="${sunY}" r="${sunR}" fill="#e9c98a"/><path d="M0 ${hy} C ${w * 0.2} ${hy - h * 0.08}, ${w * 0.35} ${hy + h * 0.06}, ${w * 0.55} ${hy - h * 0.02} S ${w * 0.85} ${hy - h * 0.1}, ${w} ${hy - h * 0.03} V ${h} H 0 Z" fill="${bottom}" opacity=".94"/>${text}</svg>`;
  }

  // ── shared fragments ─────────────────────────────────────
  function kindIcon(s, labelled = true) {
    const k = KINDS[s.kind];
    return icon(k.icon, labelled ? { label: k.label } : {});
  }
  function crumb(parts) {
    return `<div class="crumb">${parts.map((p, i) => (i ? '<span class="sep" aria-hidden="true">/</span>' : '') + p).join('')}</div>`;
  }
  function serverCrumbParts(s) {
    const k = KINDS[s.kind];
    return [`${icon(k.icon, { label: k.label })}<span>${esc(k.label)}</span>`, `<span>${esc(s.name)}</span>`];
  }
  function pageHead({ title, sub, pre = '', action = '' }) {
    return `<header class="page-head"><div class="od-field">${pre}<h1 id="page-title">${esc(title)}</h1>${sub ? `<p class="sub">${sub}</p>` : ''}</div>${action}</header>`;
  }
  function railSection(id, title, meta, cards, note = '') {
    return `<section class="section" aria-labelledby="h-${id}">
      <div class="section-head"><h2 id="h-${id}">${esc(title)}</h2>${meta ? `<span class="meta">${meta}</span>` : ''}
        <div class="rail-nav"><button type="button" class="icon-btn" data-action="rail" data-rail="${id}" data-dir="-1" aria-label="Scroll ${esc(title)} left">${icon('page-previous')}</button><button type="button" class="icon-btn" data-action="rail" data-rail="${id}" data-dir="1" aria-label="Scroll ${esc(title)} right">${icon('page-next')}</button></div>
      </div>
      <div class="rail" id="rail-${id}">${cards}</div>${note}
    </section>`;
  }
  function hiddenNote(count, names) {
    if (!count) return '';
    return `<p class="note">${icon('status-partial')}<span>${count} ${count === 1 ? 'item' : 'items'} from ${esc(names.join(', '))} hidden while offline</span></p>`;
  }

  function videoCard(v, kind, ctx, showServer) {
    const s = serverById(v.serverId);
    const left = Math.round(v.minutes * (1 - v.progress));
    const sub = `${v.episode ? esc(v.episode) : v.year} · ${v.progress > 0 ? `${left} min left` : `${v.minutes} min`}`;
    return `<button type="button" class="card ${kind === 'poster' ? 'poster' : ''}" data-action="open-video" data-id="${v.id}" data-fk="v-${v.id}-${ctx}">
      <span class="art">${art(v.palette, kind)}${kind === 'poster' && v.badge ? `<span class="badge">${esc(v.badge)}</span>` : ''}${v.watched ? `<span class="check-badge" aria-label="Watched">${icon('check')}</span>` : ''}${v.progress > 0 ? `<span class="progress"><i style="width:${(v.progress * 100).toFixed(1)}%"></i></span>` : ''}</span>
      <span class="od-field"><span class="t od-truncate">${esc(v.title)}</span><span class="s">${sub}</span>${showServer ? `<span class="src">${kindIcon(s)}<span class="od-truncate">${esc(s.name)}</span></span>` : ''}</span>
    </button>`;
  }
  function bookCard(b, ctx, showServer) {
    const s = serverById(b.serverId);
    const total = bookTotal(b);
    const playingThis = state.player.bookId === b.id;
    const meta = b.finished ? 'Finished' : b.position > 0 ? `${fmtDur(total - b.position)} left` : fmtDur(total);
    return `<button type="button" class="card square" data-action="open-book" data-id="${b.id}" data-fk="b-${b.id}-${ctx}">
      <span class="art">${art(b.palette, 'square', { title: b.title, author: b.author })}${playingThis ? `<span class="type-badge">${icon('now-playing')}<span>${state.player.playing ? 'Playing' : 'Paused'}</span></span>` : ''}${b.finished ? `<span class="check-badge" aria-label="Finished">${icon('check')}</span>` : ''}${inProgress(b) ? `<span class="progress"><i style="width:${((b.position / total) * 100).toFixed(1)}%"></i></span>` : ''}</span>
      <span class="od-field"><span class="t od-truncate">${esc(b.title)}</span><span class="s od-truncate">${esc(b.author)} · ${meta}</span>${showServer ? `<span class="src">${kindIcon(s)}<span class="od-truncate">${esc(s.name)}</span></span>` : ''}</span>
    </button>`;
  }

  function emptyOffline(s) {
    const all = !s;
    const title = all ? "Can't reach any of your servers" : `Can't reach ${s.name}`;
    const body = all
      ? 'Perfuse will keep trying. Check your network connection, or check that your servers are running.'
      : `${KINDS[s.kind].label} at ${s.address} isn't responding. It was last seen ${s.lastSeen || 'a while ago'}.`;
    return `<div class="empty" role="status">
      <span class="glyph">${icon('status-offline')}</span>
      <h2>${esc(title)}</h2><p>${esc(body)}</p>
      <div class="actions">
        <button type="button" class="btn btn-primary" data-action="${all ? 'retry-all' : 'retry-server'}" ${s ? `data-server="${s.id}"` : ''} data-fk="empty-retry">${icon('refresh')}Try again</button>
        <button type="button" class="btn btn-secondary" data-action="go-settings">${icon('settings')}Server settings</button>
      </div>
    </div>`;
  }

  // ── sidebar ──────────────────────────────────────────────
  function navItem(iconName, label, current, attrs, fk, extraCls = '') {
    return `<button type="button" class="nav-item ${extraCls}" ${attrs} data-fk="${fk}" ${current ? 'aria-current="page"' : ''}>${icon(iconName)}<span class="od-truncate">${esc(label)}</span></button>`;
  }
  function serverGroup(s) {
    const k = KINDS[s.kind];
    const v = state.view;
    const cur = v.name === 'server' && v.serverId === s.id;
    const alpha = !!state.sortAlpha[s.id];
    let libs = s.libraries.filter((l) => l.visible);
    if (alpha) libs = [...libs].sort((a, b) => a.name.localeCompare(b.name));
    const off = s.status === 'offline';
    const glyph = s.status === 'online' ? '' : statusGlyph(serverLevel(s), s.status === 'checking' ? 'Checking' : 'Offline');
    return `<section class="server-group" aria-label="${esc(k.label)} – ${esc(s.name)}">
      <div class="server-head">
        <button type="button" class="server-head-link" data-action="go-server" data-server="${s.id}" data-fk="sh-${s.id}" title="${esc(k.label)} – ${esc(s.name)}" ${cur ? 'aria-current="page"' : ''}>
          <span class="kind">${icon(k.icon, { label: k.label })}</span><span class="name od-truncate">${esc(s.name)}</span>${glyph}
        </button>
        <button type="button" class="icon-btn" data-action="toggle-sort" data-server="${s.id}" data-fk="sort-${s.id}" aria-pressed="${alpha}" aria-label="Sort ${esc(s.name)} libraries A to Z" title="Sort A–Z">${icon('sort')}</button>
        <button type="button" class="icon-btn collapse-btn" data-action="toggle-collapse" data-server="${s.id}" data-fk="col-${s.id}" aria-expanded="${!s.collapsed}" aria-controls="libs-${s.id}" aria-label="${esc(s.name)} libraries">${icon('chevron-down')}</button>
      </div>
      <div id="libs-${s.id}" ${s.collapsed ? 'hidden' : ''}>
        ${off ? `<p class="group-offline">Offline · last seen ${esc(s.lastSeen || 'a while ago')}</p>` : ''}
        ${libs.map((l) => navItem(LIB_ICONS[l.type] || 'movies', l.name, v.name === 'library' && v.libraryId === l.id && v.serverId === s.id, `data-action="go-library" data-server="${s.id}" data-lib="${l.id}"`, `lib-${l.id}`, off ? 'dim' : '')).join('')}
      </div>
    </section>`;
  }
  function statusPopover() {
    const a = aggregate();
    const rows = state.servers.map((s) => {
      const k = KINDS[s.kind];
      const lvl = serverLevel(s);
      const action = s.status === 'offline'
        ? `<button type="button" class="btn btn-secondary sm" data-action="retry-server" data-server="${s.id}" data-fk="pop-retry-${s.id}">Retry</button>`
        : statusGlyph(lvl, lvl === 'online' ? 'Connected' : 'Checking');
      return `<div class="pop-row"><span class="kind">${icon(k.icon, { label: k.label })}</span><div class="od-field od-fill"><span class="t od-truncate">${esc(s.name)}</span><span class="s ${s.status === 'offline' ? 'st-offline' : 'muted'}">${esc(serverStateText(s))}</span></div>${action}</div>`;
    }).join('');
    return `<div class="popover" id="status-pop" role="dialog" aria-label="Server status">
      <h3>${esc(a.title)}</h3><p class="summary">${esc(a.summary)}</p>${rows}
      <div class="pop-foot"><button type="button" class="btn btn-text sm" data-action="retry-all" data-fk="pop-retry-all">${icon('refresh')}Check again</button><button type="button" class="btn btn-text sm" data-action="go-settings" data-fk="pop-manage">Manage servers</button></div>
    </div>`;
  }
  function renderSidebar() {
    const v = state.view;
    const a = aggregate();
    const scroll = $('.side-scroll', $('#sidebar'));
    const top = scroll ? scroll.scrollTop : 0;
    $('#sidebar').innerHTML = `
      <div class="sidebar-top">${navItem('home', 'Home', v.name === 'home', 'data-action="go-home"', 'nav-home')}</div>
      <div class="side-scroll">${state.servers.map(serverGroup).join('')}</div>
      <div class="sidebar-foot" id="sidebar-foot">
        <button type="button" class="status-btn st-${a.level}" id="status-btn" data-action="toggle-status" data-fk="status" aria-expanded="${state.status.open}" aria-controls="status-pop">${statusGlyph(a.level)}<span class="label">${esc(a.text)}</span></button>
        ${state.status.open ? statusPopover() : ''}
        ${navItem('settings', 'Servers & settings', v.name === 'settings', 'data-action="go-settings"', 'nav-settings')}
      </div>`;
    const ns = $('.side-scroll', $('#sidebar'));
    if (ns) ns.scrollTop = top;
  }

  // ── views ────────────────────────────────────────────────
  function viewHome() {
    const a = aggregate();
    const refresh = `<button type="button" class="icon-btn lg" data-action="retry-all" data-fk="home-refresh" aria-label="Check all servers" title="Check all servers">${icon('refresh')}</button>`;
    let h = pageHead({ title: 'Home', sub: 'Pick up where you left off, across all your servers.', action: refresh });
    h += `<section class="section" aria-labelledby="h-servers"><div class="section-head"><h2 id="h-servers">Servers</h2><span class="meta st-${a.level}">${esc(a.text)}</span></div><div class="server-cards">${state.servers.map(serverCard).join('')}</div></section>`;
    if (a.level === 'offline') return h + emptyOffline(null);

    const vids = state.videos.filter((v) => v.progress > 0 && !v.watched && itemVisible(v));
    const vOn = vids.filter((v) => isOnline(serverById(v.serverId)));
    const vOffNames = [...new Set(vids.filter((v) => !isOnline(serverById(v.serverId))).map((v) => serverById(v.serverId).name))];
    if (vids.length) h += railSection('cw', 'Continue Watching', `${vOn.length}`, vOn.map((v) => videoCard(v, 'landscape', 'home', true)).join(''), hiddenNote(vids.length - vOn.length, vOffNames));

    const books = state.books.filter((b) => inProgress(b) && itemVisible(b));
    const bOn = books.filter((b) => isOnline(serverById(b.serverId)));
    const bOffNames = [...new Set(books.filter((b) => !isOnline(serverById(b.serverId))).map((b) => serverById(b.serverId).name))];
    if (books.length) h += railSection('cl', 'Continue Listening', `${bOn.length}`, bOn.map((b) => bookCard(b, 'home', true)).join(''), hiddenNote(books.length - bOn.length, bOffNames));
    return h;
  }
  function serverCard(s) {
    const k = KINDS[s.kind];
    const lvl = serverLevel(s);
    const libs = s.libraries.filter((l) => l.visible);
    return `<button type="button" class="server-card ${s.status === 'offline' ? 'offline' : ''}" data-action="go-server" data-server="${s.id}" data-fk="card-${s.id}">
      <span class="kind">${icon(k.icon, { label: k.label })}</span>
      <span class="od-field"><span class="name od-truncate">${esc(s.name)}</span><span class="caption">${esc(k.label)}</span></span>
      <span class="state st-${lvl}" style="grid-column:2">${statusGlyph(lvl)}<span>${esc(s.status === 'online' ? `Connected · ${s.latencyMs} ms` : s.status === 'checking' ? 'Checking…' : `Offline · last seen ${s.lastSeen || 'a while ago'}`)}</span></span>
      <span class="libs">${libs.map((l) => `<span class="chip">${icon(LIB_ICONS[l.type] || 'movies')}<span>${esc(l.name)}</span></span>`).join('')}</span>
    </button>`;
  }

  function viewServer(s) {
    if (!s) return viewHome();
    const k = KINDS[s.kind];
    const pre = crumb(serverCrumbParts(s));
    const refresh = `<button type="button" class="icon-btn lg" data-action="retry-server" data-server="${s.id}" data-fk="srv-refresh" aria-label="Refresh ${esc(s.name)}" title="Refresh">${icon('refresh')}</button>`;
    const libs = s.libraries.filter((l) => l.visible);
    const sub = `${esc(k.label)} server · ${libs.length} ${libs.length === 1 ? 'library' : 'libraries'}`;
    let h = pageHead({ title: s.name, sub, pre, action: refresh });
    if (s.status !== 'online') return h + emptyOffline(s);
    if (s.kind === 'audiobookshelf') {
      const books = state.books.filter((b) => b.serverId === s.id && itemVisible(b));
      const cont = books.filter(inProgress);
      if (cont.length) h += railSection('srv-cl', 'Continue Listening', `${cont.length}`, cont.map((b) => bookCard(b, 'srv', false)).join(''));
      const recent = [...books].sort((a, b) => a.recent - b.recent);
      h += railSection('srv-ra', 'Recently Added', `1–${recent.length} of ${recent.length}`, recent.map((b) => bookCard(b, 'srv-ra', false)).join(''));
    } else {
      const vids = state.videos.filter((v) => v.serverId === s.id && itemVisible(v));
      const cont = vids.filter((v) => v.progress > 0 && !v.watched);
      if (cont.length) h += railSection('srv-cw', 'Continue Watching', `${cont.length}`, cont.map((v) => videoCard(v, 'landscape', 'srv', false)).join(''));
      const recent = [...vids].sort((a, b) => a.recent - b.recent);
      h += railSection('srv-ra', 'Recently Added', `1–${recent.length} of ${recent.length}`, recent.map((v) => videoCard(v, 'poster', 'srv-ra', false)).join(''));
    }
    return h;
  }

  function viewLibrary(s, l) {
    if (!s || !l) return viewHome();
    const pre = crumb([...serverCrumbParts(s)]);
    let h;
    if (s.status !== 'online') return pageHead({ title: l.name, pre }) + emptyOffline(s);
    if (l.type === 'audiobooks') {
      const books = state.books.filter((b) => b.libraryId === l.id && b.serverId === s.id).sort((a, b) => a.title.localeCompare(b.title));
      h = pageHead({ title: l.name, sub: `${books.length} audiobooks`, pre });
      h += `<div class="grid-lib books">${books.map((b) => bookCard(b, 'lib', false)).join('')}</div>`;
    } else {
      const vids = state.videos.filter((v) => v.libraryId === l.id && v.serverId === s.id).sort((a, b) => a.title.localeCompare(b.title));
      h = pageHead({ title: l.name, sub: `${vids.length} ${l.type === 'shows' ? 'shows' : 'titles'}`, pre });
      h += vids.length
        ? `<div class="grid-lib">${vids.map((v) => videoCard(v, 'poster', 'lib', false)).join('')}</div>`
        : `<div class="empty"><h2>Nothing here yet</h2><p>When ${esc(s.name)} adds titles to ${esc(l.name)}, they'll appear here.</p></div>`;
    }
    return h;
  }

  function viewVideo(v) {
    if (!v) return viewHome();
    const s = serverById(v.serverId);
    const l = libOf(s, v.libraryId);
    const pre = crumb([...serverCrumbParts(s), `<span>${esc(l.name)}</span>`]);
    const left = Math.round(v.minutes * (1 - v.progress));
    return `<div style="margin-bottom:24px">${pre}</div>
      <section class="hero video" aria-labelledby="page-title">
        <span class="art">${art(v.palette, 'poster', { alt: `Poster for ${v.title}` })}</span>
        <div class="od-field"><span class="eyebrow">Original quality</span><h1 id="page-title">${esc(v.title)}</h1>
          <div class="meta-line"><span>${v.year}</span><span>${v.minutes} min</span>${v.episode ? `<span>${esc(v.episode)}</span>` : ''}<span>${esc(v.badge)}</span></div></div>
      </section>
      <div class="actions-row">
        ${v.progress > 0 ? `<button type="button" class="btn btn-primary" data-action="video-play" data-id="${v.id}" data-fk="v-resume">${icon('resume')}Resume</button>` : ''}
        <button type="button" class="btn ${v.progress > 0 ? 'btn-secondary' : 'btn-primary'}" data-action="video-play" data-id="${v.id}" data-fk="v-start">${icon(v.progress > 0 ? 'play-from-beginning' : 'play')}${v.progress > 0 ? 'Play from beginning' : 'Play'}</button>
        ${v.progress > 0 ? `<div class="progress-line"><span class="bar"><i style="width:${(v.progress * 100).toFixed(1)}%"></i></span><span class="caption">${left} min left</span></div>` : ''}
      </div>`;
  }

  function chapterList(b, ctx) {
    const ci = chapterAt(b, b.position);
    const active = inProgress(b) || state.player.bookId === b.id;
    return `<ol class="chapter-list">${b.chapters.map((c, i) => {
      const done = b.finished || (active && i < ci);
      const cur = active && i === ci && !b.finished;
      const pct = cur ? ((b.position - c.start) / c.duration) * 100 : 0;
      const n = done ? icon('check', { label: 'Played' }) : cur ? icon('now-playing', { label: 'Current chapter' }) : `${i + 1}`;
      return `<li><button type="button" class="chapter ${done ? 'done' : ''} ${cur ? 'current' : ''}" data-action="play-chapter" data-id="${b.id}" data-index="${i}" data-fk="ch-${ctx}-${i}" ${cur ? 'aria-current="true"' : ''}>
        <span class="n">${n}</span>
        <span class="od-field"><span class="t od-truncate">${esc(c.title)}</span>${cur ? `<span class="mini" aria-hidden="true"><i data-live-w="chap" style="width:${pct.toFixed(1)}%"></i></span>` : ''}</span>
        <span class="dur">${fmtClock(c.duration)}</span>
      </button></li>`;
    }).join('')}</ol>`;
  }
  function bookmarkList(b, ctx) {
    if (!b.bookmarks.length) return `<p class="side-empty">No bookmarks yet. Add one while listening to save your place with a note.</p>`;
    return [...b.bookmarks].sort((x, y) => x.at - y.at).map((m) => {
      const c = b.chapters[chapterAt(b, m.at)];
      return `<div class="bm">${icon('bookmark')}<button type="button" class="jump od-field" data-action="bm-jump" data-id="${b.id}" data-bm="${m.id}" data-fk="bm-${ctx}-${m.id}"><span class="od-truncate">${esc(m.note)}</span><span class="s">${fmtClock(m.at)} · ${esc(c.title)}</span></button><button type="button" class="icon-btn" data-action="bm-delete" data-id="${b.id}" data-bm="${m.id}" aria-label="Delete bookmark ${esc(m.note)}">${icon('close')}</button></div>`;
    }).join('');
  }

  function viewBook(b) {
    if (!b) return viewHome();
    const s = serverById(b.serverId);
    const l = libOf(s, b.libraryId);
    const total = bookTotal(b);
    const ci = chapterAt(b, b.position);
    const pre = crumb([...serverCrumbParts(s), `<span>${esc(l.name)}</span>`]);
    const isCur = state.player.bookId === b.id;
    const playing = isCur && state.player.playing;
    let primary;
    if (playing) primary = `<button type="button" class="btn btn-primary" data-action="toggle-play" data-fk="bk-primary">${icon('pause')}Pause</button>`;
    else if (inProgress(b)) primary = `<button type="button" class="btn btn-primary" data-action="book-play" data-id="${b.id}" data-fk="bk-primary">${icon('resume')}Resume</button>`;
    else primary = `<button type="button" class="btn btn-primary" data-action="book-start" data-id="${b.id}" data-fk="bk-primary">${icon('play')}${b.finished ? 'Listen again' : 'Play'}</button>`;
    const secondary = inProgress(b) ? `<button type="button" class="btn btn-secondary" data-action="book-start" data-id="${b.id}" data-fk="bk-start">${icon('play-from-beginning')}Play from beginning</button>` : '';
    const prog = inProgress(b)
      ? `<div class="progress-line"><span class="bar"><i style="width:${((b.position / total) * 100).toFixed(1)}%"></i></span><span class="caption">Chapter ${ci + 1} of ${b.chapters.length} · ${fmtDur(total - b.position)} left</span></div>`
      : b.finished ? '<span class="caption">Finished</span>' : '';
    const series = b.series ? `<span>${esc(b.series)}, book ${b.seriesIndex}</span>` : '';
    return `<div style="margin-bottom:24px">${pre}</div>
      <section class="hero" aria-labelledby="page-title">
        <span class="art">${art(b.palette, 'square', { title: b.title, author: b.author, alt: `Cover of ${b.title}` })}</span>
        <div class="od-field"><span class="eyebrow">Audiobook</span><h1 id="page-title">${esc(b.title)}</h1>
          <p class="byline">By <b>${esc(b.author)}</b> · Narrated by <b>${esc(b.narrator)}</b></p>
          <div class="meta-line">${series}<span>${b.year}</span><span>${fmtDur(total)}</span><span>${b.chapters.length} chapters</span></div>
          <p class="desc">${esc(b.description)}</p></div>
      </section>
      <div class="actions-row">${primary}${secondary}${isCur ? `<button type="button" class="btn btn-secondary" data-action="open-player" data-fk="bk-open">${icon('headphones')}Open player</button>` : ''}${prog}</div>
      <div class="two-col">
        <section class="panel" aria-labelledby="h-chapters"><h2 id="h-chapters">Chapters</h2>${chapterList(b, 'detail')}</section>
        <div class="od-stack" style="--od-gap:16px">
          <section class="panel" aria-labelledby="h-bm"><h2 id="h-bm">Bookmarks</h2>${bookmarkList(b, 'detail')}</section>
          <section class="panel" aria-labelledby="h-info"><h2 id="h-info">Details</h2>
            <dl class="kv"><dt>Server</dt><dd>${esc(KINDS[s.kind].label)} · ${esc(s.name)}</dd><dt>Library</dt><dd>${esc(l.name)}</dd><dt>Narrator</dt><dd>${esc(b.narrator)}</dd><dt>Released</dt><dd>${b.year}</dd><dt>Length</dt><dd>${fmtClock(total)}</dd></dl>
          </section>
        </div>
      </div>`;
  }

  function menuHTML(where) {
    const m = state.menu;
    if (!m || m.where !== where) return '';
    const p = state.player;
    const cls = where === 'dock' ? 'up' : 'down';
    if (m.type === 'speed') {
      return `<div class="menu ${cls}" role="menu" aria-label="Playback speed" id="menu"><h4>Speed</h4>${SPEEDS.map((x) => `<button type="button" role="menuitemradio" aria-checked="${p.speed === x}" data-action="set-speed" data-value="${x}"><span class="tick">${p.speed === x ? icon('check') : ''}</span>${fmtSpeed(x)}${x === 1 ? ' (normal)' : ''}</button>`).join('')}</div>`;
    }
    const sl = p.sleep;
    const item = (val, label, on) => `<button type="button" role="menuitemradio" aria-checked="${on}" data-action="set-sleep" data-value="${val}"><span class="tick">${on ? icon('check') : ''}</span>${label}</button>`;
    return `<div class="menu ${cls}" role="menu" aria-label="Sleep timer" id="menu"><h4>Sleep timer</h4>${item('off', 'Off', !sl)}${SLEEP_MINUTES.map((x) => item(x, `${x} minutes`, !!sl && sl.mode === 'minutes' && sl.minutes === x)).join('')}${item('chapter', 'End of chapter', !!sl && sl.mode === 'chapter')}</div>`;
  }
  function sleepLabel() {
    const sl = state.player.sleep;
    if (!sl) return '';
    return sl.mode === 'chapter' ? 'End of chapter' : fmtClock(sl.remaining);
  }

  function viewPlayer() {
    const p = state.player;
    const b = bookById(p.bookId);
    if (!b) return viewHome();
    const s = serverById(b.serverId);
    const l = libOf(s, b.libraryId);
    const total = bookTotal(b);
    const ci = chapterAt(b, b.position);
    const c = b.chapters[ci];
    const ticks = b.chapters.slice(1).map((ch) => `<span class="tick ${ch.index === ci ? 'cur' : ''}" style="left:${((ch.start / total) * 100).toFixed(2)}%"></span>`).join('');
    const tab = p.tab;
    return `<div class="player">
      <div class="player-main">
        <div class="player-top"><button type="button" class="icon-btn" data-action="back" data-fk="pl-collapse" aria-label="Minimise player" title="Minimise player">${icon('player-collapse')}</button><div class="od-fill">${crumb([...serverCrumbParts(s), `<span>${esc(l.name)}</span>`])}</div></div>
        <div class="player-cover">
          <span class="art">${art(b.palette, 'square', { title: b.title, author: b.author, alt: `Cover of ${b.title}` })}</span>
          <div class="od-field"><span class="eyebrow">Chapter ${ci + 1} of ${b.chapters.length}</span><span class="chap-now">${esc(c.title)}</span><h1 id="page-title">${esc(b.title)}</h1><span class="muted">By ${esc(b.author)} · Narrated by ${esc(b.narrator)}</span></div>
        </div>
        <div class="scrub">
          <label class="visually-hidden" for="seek">Position in ${esc(c.title)}</label>
          <input id="seek" class="seek" type="range" min="0" max="${c.duration}" step="1" value="${Math.floor(b.position - c.start)}" data-fk="seek" style="--pct:${(((b.position - c.start) / c.duration) * 100).toFixed(2)}%" aria-valuetext="${fmtClock(b.position - c.start)} of ${fmtClock(c.duration)}">
          <div class="lbls"><b data-live="chap-elapsed">${fmtClock(b.position - c.start)}</b><span data-live="chap-remaining">−${fmtClock(c.start + c.duration - b.position)}</span></div>
        </div>
        <div class="scrub">
          <div class="book-track" aria-hidden="true"><div class="line"><i data-live-w="book" style="width:${((b.position / total) * 100).toFixed(2)}%"></i></div>${ticks}</div>
          <div class="lbls"><span>Book <b data-live="book-elapsed">${fmtClock(b.position)}</b> of ${fmtClock(total)}</span><span data-live="book-left">${bookLeftText(b)}</span></div>
        </div>
        <div class="big-transport">
          <button type="button" class="icon-btn lg" data-action="prev-chapter" data-fk="pl-prev" aria-label="Previous chapter" title="Previous chapter">${icon('chapter-previous')}</button>
          <button type="button" class="icon-btn lg" data-action="skip" data-value="-30" data-fk="pl-back" aria-label="Back 30 seconds" title="Back 30 seconds">${icon('back-30')}</button>
          <button type="button" class="icon-btn xl" data-action="toggle-play" data-fk="pl-play" aria-label="${p.playing ? 'Pause' : 'Play'}">${icon(p.playing ? 'pause' : 'play')}</button>
          <button type="button" class="icon-btn lg" data-action="skip" data-value="30" data-fk="pl-fwd" aria-label="Forward 30 seconds" title="Forward 30 seconds">${icon('forward-30')}</button>
          <button type="button" class="icon-btn lg" data-action="next-chapter" data-fk="pl-next" aria-label="Next chapter" title="Next chapter" ${ci >= b.chapters.length - 1 ? 'disabled' : ''}>${icon('chapter-next')}</button>
        </div>
        <div class="player-tools">
          <span class="menu-anchor"><button type="button" class="tool ${p.speed !== 1 ? 'on' : ''}" data-action="menu" data-menu="speed" data-where="player" data-fk="pl-speed" aria-haspopup="menu" aria-expanded="${state.menu?.type === 'speed' && state.menu.where === 'player'}">${icon('speed')}<span>Speed ${fmtSpeed(p.speed)}</span></button>${state.menu?.type === 'speed' ? menuHTML('player') : ''}</span>
          <span class="menu-anchor"><button type="button" class="tool ${p.sleep ? 'on' : ''}" data-action="menu" data-menu="sleep" data-where="player" data-fk="pl-sleep" aria-haspopup="menu" aria-expanded="${state.menu?.type === 'sleep' && state.menu.where === 'player'}">${icon('sleep-timer')}<span>Sleep${p.sleep ? ' · ' : ''}<span data-live="sleep">${sleepLabel()}</span></span></button>${state.menu?.type === 'sleep' ? menuHTML('player') : ''}</span>
          <button type="button" class="tool" data-action="bm-add" data-fk="pl-bm">${icon('bookmark-add')}<span>Add bookmark</span></button>
        </div>
      </div>
      <aside class="player-side" aria-label="Chapters and bookmarks">
        <div class="tabs" role="tablist" aria-label="Player panel">
          <button type="button" role="tab" id="tab-chapters" aria-controls="panel-side" aria-selected="${tab === 'chapters'}" data-action="tab" data-value="chapters" data-fk="tab-chapters">Chapters (${b.chapters.length})</button>
          <button type="button" role="tab" id="tab-bookmarks" aria-controls="panel-side" aria-selected="${tab === 'bookmarks'}" data-action="tab" data-value="bookmarks" data-fk="tab-bookmarks">Bookmarks (${b.bookmarks.length})</button>
        </div>
        <div class="side-list" id="panel-side" role="tabpanel" aria-labelledby="tab-${tab}">${tab === 'chapters' ? chapterList(b, 'player') : bookmarkList(b, 'player')}</div>
      </aside>
    </div>`;
  }
  function bookLeftText(b) {
    const left = bookTotal(b) - b.position;
    const sp = state.player.speed;
    return sp === 1 ? `${fmtDur(left)} left` : `${fmtDur(left / sp)} left at ${fmtSpeed(sp)}`;
  }

  function renderDock() {
    const p = state.player;
    const b = bookById(p.bookId);
    if (!b || state.view.name === 'player' || state.onboarding || !state.servers.length) return '';
    const c = b.chapters[chapterAt(b, b.position)];
    const pct = ((b.position - c.start) / c.duration) * 100;
    return `<section class="dock" aria-label="Now playing">
      <button type="button" class="now" data-action="open-player" data-fk="dock-now" aria-label="Open player: ${esc(b.title)}, ${esc(c.title)}">
        <span class="art">${art(b.palette, 'square')}</span>
        <span class="od-field"><span class="t od-truncate">${esc(b.title)}</span><span class="s od-truncate" data-live="dock-chap">${esc(c.title)}</span></span>
      </button>
      <div class="center">
        <div class="transport">
          <button type="button" class="icon-btn" data-action="prev-chapter" data-fk="dk-prev" aria-label="Previous chapter" title="Previous chapter">${icon('chapter-previous')}</button>
          <button type="button" class="icon-btn" data-action="skip" data-value="-30" data-fk="dk-back" aria-label="Back 30 seconds" title="Back 30 seconds">${icon('back-30')}</button>
          <button type="button" class="icon-btn play" data-action="toggle-play" data-fk="dk-play" aria-label="${p.playing ? 'Pause' : 'Play'}">${icon(p.playing ? 'pause' : 'play')}</button>
          <button type="button" class="icon-btn" data-action="skip" data-value="30" data-fk="dk-fwd" aria-label="Forward 30 seconds" title="Forward 30 seconds">${icon('forward-30')}</button>
          <button type="button" class="icon-btn" data-action="next-chapter" data-fk="dk-next" aria-label="Next chapter" title="Next chapter">${icon('chapter-next')}</button>
        </div>
        <div class="time-row" aria-hidden="true"><span data-live="dock-l">${fmtClock(b.position - c.start)}</span><span class="track"><i data-live-w="chap" style="width:${pct.toFixed(1)}%"></i></span><span data-live="dock-r">−${fmtClock(c.start + c.duration - b.position)}</span></div>
      </div>
      <div class="side">
        ${p.sleep ? `<span class="sleep-badge" title="Sleep timer">${icon('sleep-timer')} <span data-live="sleep">${sleepLabel()}</span></span>` : ''}
        <span class="menu-anchor"><button type="button" class="speed-chip ${p.speed !== 1 ? 'on' : ''}" data-action="menu" data-menu="speed" data-where="dock" data-fk="dk-speed" aria-haspopup="menu" aria-label="Playback speed ${fmtSpeed(p.speed)}">${fmtSpeed(p.speed)}</button>${state.menu?.type === 'speed' ? menuHTML('dock') : ''}</span>
        <span class="menu-anchor"><button type="button" class="icon-btn ${p.sleep ? 'on' : ''}" data-action="menu" data-menu="sleep" data-where="dock" data-fk="dk-sleep" aria-haspopup="menu" aria-label="Sleep timer">${icon('sleep-timer')}</button>${state.menu?.type === 'sleep' ? menuHTML('dock') : ''}</span>
        <button type="button" class="icon-btn" data-action="open-player" data-tab="chapters" data-fk="dk-chapters" aria-label="Chapters" title="Chapters">${icon('chapters')}</button>
        <button type="button" class="icon-btn" data-action="open-player" data-fk="dk-expand" aria-label="Open player" title="Open player">${icon('player-expand')}</button>
        <button type="button" class="icon-btn" data-action="stop-player" data-fk="dk-close" aria-label="Stop and close player" title="Stop">${icon('close')}</button>
      </div>
    </section>`;
  }

  // ── settings ─────────────────────────────────────────────
  function viewSettings() {
    const n = state.servers.length;
    let h = pageHead({ title: 'Settings', sub: 'Your servers and playback preferences' });
    h += `<section class="settings-section" aria-labelledby="h-srv">
      <div class="section-head"><h2 id="h-srv">Servers</h2><span class="meta">${n} ${n === 1 ? 'server' : 'servers'}</span><span class="od-fill"></span>
        <button type="button" class="btn btn-primary sm" data-action="add-server" data-fk="add-server">${icon('add')}Add server</button></div>
      <ul class="server-list" id="server-list">${state.servers.map((s, i) => serverRow(s, i, n)).join('')}</ul>
      <p class="settings-note" id="reorder-help">Drag servers, or focus a handle and press the up and down arrow keys, to change their order in the sidebar and on Home.</p>
    </section>`;
    return h;
  }
  function serverRow(s, i, n) {
    const k = KINDS[s.kind];
    const lvl = serverLevel(s);
    const open = state.expanded.has(s.id);
    const renaming = state.renaming === s.id;
    const name = renaming
      ? `<label class="visually-hidden" for="rename-${s.id}">Server name</label><input class="rename-input" id="rename-${s.id}" data-rename="${s.id}" data-fk="rename-${s.id}" value="${esc(s.name)}" maxlength="40">`
      : `<span class="name od-truncate">${esc(s.name)}</span>`;
    const stateText = s.status === 'online' ? `Connected · ${s.latencyMs} ms` : s.status === 'checking' ? 'Checking…' : 'Offline';
    return `<li class="srow" draggable="${!renaming}" data-row="${s.id}">
      <div class="srow-main">
        <button type="button" class="handle" data-action="noop" data-handle="${s.id}" data-fk="handle-${s.id}" aria-label="Reorder ${esc(s.name)}, position ${i + 1} of ${n}" aria-describedby="reorder-help">${icon('drag-handle')}</button>
        <span class="kind">${icon(k.icon, { label: k.label })}</span>
        <div class="od-field">${name}<span class="addr od-truncate">${esc(k.label)} · ${esc(s.address)}</span></div>
        <span class="state st-${lvl}">${statusGlyph(lvl)}<span>${stateText}</span></span>
        <div class="row-actions">
          ${s.status === 'offline' ? `<button type="button" class="btn btn-secondary sm" data-action="retry-server" data-server="${s.id}" data-fk="row-retry-${s.id}">${icon('refresh')}Reconnect</button>` : ''}
          <button type="button" class="icon-btn" data-action="rename" data-server="${s.id}" data-fk="row-rename-${s.id}" aria-label="Rename ${esc(s.name)}" title="Rename">${icon('rename')}</button>
          <button type="button" class="icon-btn collapse-btn" data-action="toggle-row" data-server="${s.id}" data-fk="row-open-${s.id}" aria-expanded="${open}" aria-controls="detail-${s.id}" aria-label="${esc(s.name)} details" title="Details">${icon('chevron-down')}</button>
        </div>
      </div>
      <div class="srow-detail" id="detail-${s.id}" ${open ? '' : 'hidden'}>
        <dl class="kv">
          <dt>Type</dt><dd>${esc(k.label)}</dd>
          <dt>Address</dt><dd>${esc(s.address)}</dd>
          <dt>Signed in as</dt><dd>${esc(s.account)}</dd>
          <dt>Status</dt><dd class="st-${lvl}">${esc(serverStateText(s))}</dd>
        </dl>
        <div><p class="caption" style="margin-bottom:4px">Libraries shown in the sidebar and on Home</p>
          <div class="lib-toggles">${s.libraries.map((l) => `<label class="lib-toggle">${icon(LIB_ICONS[l.type] || 'movies')}<span class="od-fill">${esc(l.name)}</span><span class="switch"><input type="checkbox" role="switch" data-libtoggle="${l.id}" data-server="${s.id}" data-fk="lt-${l.id}" ${l.visible ? 'checked' : ''} aria-label="Show ${esc(l.name)}"><i></i></span></label>`).join('')}</div>
        </div>
        <div class="form-actions">
          <button type="button" class="btn btn-secondary sm" data-action="retry-server" data-server="${s.id}" data-fk="row-check-${s.id}">${icon('refresh')}Check connection</button>
          <button type="button" class="btn btn-danger sm" data-action="remove-server" data-server="${s.id}" data-fk="row-remove-${s.id}">${icon('forget')}Sign out and remove</button>
        </div>
      </div>
    </li>`;
  }

  // ── add-server flow (shared by first-run landing and the Settings dialog) ──
  const REQUIRED = {
    'plex-manual': ['address', 'token'],
    jellyfin: ['address', 'username'],
    audiobookshelf: ['address', 'username', 'password'],
  };
  function newFlow(kind = null) {
    return { step: kind ? 'form' : 'kind', kind, values: { address: '', username: '', password: '', token: '', remember: true }, errors: {}, banner: null, busy: false, picks: [], results: [], timer: null };
  }
  function findSeedByAddress(addr) {
    const a = normAddr(addr);
    return window.SEED_SERVERS.find((s) => normAddr(s.address) === a);
  }
  const normAddr = (a) => String(a || '').trim().replace(/\/+$/, '').toLowerCase();

  function validate(name) {
    const f = state.flow;
    const v = (f.values[name] || '').trim();
    if (name === 'address') {
      if (!v) return "Enter your server's address.";
      let url;
      try { url = new URL(v); } catch { url = null; }
      if (!url || !/^https?:$/.test(url.protocol)) return 'Start the address with http:// or https://, for example http://192.168.1.30:8096.';
      const dup = state.servers.find((s) => normAddr(s.address) === normAddr(v));
      if (dup) return `This server is already added as “${dup.name}”.`;
    }
    if (name === 'username' && !v) return 'Enter your username.';
    if (name === 'password' && !v && f.kind === 'audiobookshelf') return 'Enter your password.';
    if (name === 'token' && !v) return 'Enter the access token for this Plex server.';
    return '';
  }
  function setFieldError(name, msg) {
    state.flow.errors[name] = msg || undefined;
    const wrap = document.querySelector(`[data-fieldwrap="${name}"]`);
    if (!wrap) return;
    const input = wrap.querySelector('input');
    const err = wrap.querySelector('.err');
    wrap.classList.toggle('invalid', !!msg);
    if (input) { if (msg) input.setAttribute('aria-invalid', 'true'); else input.removeAttribute('aria-invalid'); }
    if (err) { err.hidden = !msg; const t = err.querySelector('span'); if (t) t.textContent = msg || ''; }
  }

  function field(name, label, { type = 'text', required = false, help = '', autocomplete = 'off' } = {}) {
    const f = state.flow;
    const err = f.errors[name];
    const id = `f-${name}`;
    return `<div class="field ${err ? 'invalid' : ''}" data-fieldwrap="${name}">
      <label for="${id}">${esc(label)}${required ? ' <span class="req">(required)</span>' : ''}</label>
      <input id="${id}" type="${type}" data-field="${name}" data-fk="${id}" value="${esc(f.values[name])}" ${required ? 'aria-required="true"' : ''} aria-describedby="${help ? `${id}-help ` : ''}${id}-err" ${err ? 'aria-invalid="true"' : ''} autocomplete="${autocomplete}" spellcheck="false">
      ${help ? `<p class="help" id="${id}-help">${help}</p>` : ''}
      <p class="err" id="${id}-err" ${err ? '' : 'hidden'}>${icon('alert')}<span>${esc(err || '')}</span></p>
    </div>`;
  }
  function bannerHTML() {
    const b = state.flow.banner;
    if (!b) return '';
    const list = b.fields ? `<span class="od-cluster" style="--od-gap:4px 12px">${b.fields.map((x) => `<button type="button" class="btn btn-text sm" data-action="focus-field" data-field="${x.name}" style="padding:0;height:auto">${esc(x.msg)}</button>`).join('')}</span>` : '';
    return `<div class="banner ${b.type}" role="alert" id="flow-banner" tabindex="-1">${icon(b.type === 'error' ? 'alert' : 'status-partial')}<div class="od-field"><strong style="font-weight:400">${esc(b.title)}</strong>${b.text ? `<span>${esc(b.text)}</span>` : ''}${list}</div></div>`;
  }
  function flowHead(title, { back = true } = {}) {
    const f = state.flow;
    const k = f.kind ? KINDS[f.kind] : null;
    const close = state.flowCtx === 'dialog' ? `<button type="button" class="icon-btn" data-action="flow-close" data-fk="flow-close" aria-label="Close">${icon('close')}</button>` : '';
    const backBtn = back ? `<button type="button" class="icon-btn" data-action="flow-back" data-fk="flow-back" aria-label="Back to server types">${icon('back')}</button>` : '';
    return `<div class="flow-head">${backBtn}${k ? `<span class="kind">${icon(k.icon, { label: k.label })}</span>` : ''}<h2 id="${state.flowCtx === 'dialog' ? 'add-dialog-title' : 'flow-title'}">${esc(title)}</h2>${close}</div>`;
  }
  function rememberLine() {
    return `<label class="checkline"><input type="checkbox" data-field-check="remember" ${state.flow.values.remember ? 'checked' : ''}>Remember me on this computer</label>`;
  }
  function connectBtn(label) {
    const f = state.flow;
    return `<button type="button" class="btn btn-primary" data-action="flow-connect" data-fk="flow-connect" ${f.busy ? 'disabled aria-busy="true"' : ''}>${f.busy ? '<span class="spinner" aria-hidden="true"></span>Connecting…' : `${icon('server')}${esc(label)}`}</button>`;
  }
  function kindOption(kind, text) {
    const k = KINDS[kind];
    return `<button type="button" class="kind-opt" data-action="flow-kind" data-kind="${kind}" data-fk="kind-${kind}"><span class="kind">${icon(k.icon)}</span><span class="t">${esc(k.label)}</span><span class="s">${esc(text)}</span></button>`;
  }
  function addedList() {
    if (!state.servers.length) return '';
    return `<div class="added-list" aria-label="Connected servers">${state.servers.map((s) => `<div class="added">${kindIcon(s)}<div class="od-field od-fill"><span class="od-truncate">${esc(s.name)}</span><span class="s">${esc(KINDS[s.kind].label)} · ${s.libraries.length} ${s.libraries.length === 1 ? 'library' : 'libraries'}</span></div><span class="ok">${icon('check', { label: 'Connected' })}</span></div>`).join('')}</div>`;
  }

  function flowHTML() {
    const f = state.flow;
    const ctx = state.flowCtx;
    if (f.step === 'kind') {
      const kinds = `<div class="kind-grid">${kindOption('plex', 'Movies and TV. Sign in with your Plex account.')}${kindOption('jellyfin', 'Movies and TV. Use your server address and Jellyfin account.')}${kindOption('audiobookshelf', 'Audiobooks and podcasts, with chapters and bookmarks.')}</div>`;
      if (ctx === 'dialog') {
        return `${flowHead('Add a server', { back: false })}<p class="muted" style="margin-bottom:20px">What kind of server are you connecting to?</p>${kinds}`;
      }
      if (!state.servers.length) {
        return `<h1 id="flow-title">Make yourself at home.</h1><p class="lead">Your films, shows and audiobooks, in original quality. Connect a server to get started. You can add more later.</p>${kinds}`;
      }
      return `<h1 id="flow-title">Add another server</h1><p class="lead">Connected so far:</p>${addedList()}${kinds}<div class="form-actions" style="margin-top:28px"><button type="button" class="btn btn-primary" data-action="flow-finish" data-fk="flow-finish">Continue to Perfuse</button></div>`;
    }
    if (f.step === 'form' && f.kind === 'plex') {
      return `${flowHead('Connect to Plex')}<div class="form">
        ${bannerHTML()}
        <p class="muted">Sign in securely in your browser, then choose which of your Plex servers to add.</p>
        <div class="form-actions"><button type="button" class="btn btn-primary" data-action="plex-signin" data-fk="plex-signin">${icon('sign-in-browser')}Sign in with Plex</button></div>
        ${rememberLine()}
        <details class="manual" ${f.manualOpen ? 'open' : ''} data-manual><summary data-fk="plex-manual">${icon('chevron-down')}Connect manually with an address and token</summary>
          <div class="form">${field('address', 'Server address', { type: 'url', required: true, help: 'For example https://192.168.1.20:32400' })}${field('token', 'Access token', { type: 'password', required: true })}<div class="form-actions">${connectBtn('Connect')}</div></div>
        </details>
      </div>`;
    }
    if (f.step === 'form') {
      const jf = f.kind === 'jellyfin';
      return `${flowHead(`Connect to ${KINDS[f.kind].label}`)}<div class="form">
        ${bannerHTML()}
        ${field('address', 'Server address', { type: 'url', required: true, help: jf ? 'For example http://192.168.1.30:8096' : 'For example https://abs.example.com' })}
        <div class="field-row">${field('username', 'Username', { required: true, autocomplete: 'username' })}${field('password', 'Password', { type: 'password', required: !jf, autocomplete: 'current-password', help: jf ? 'Leave empty if your account has no password.' : '' })}</div>
        ${rememberLine()}
        <div class="form-actions">${connectBtn('Connect')}</div>
        ${jf ? `<div class="or">or</div><div class="form-actions"><button type="button" class="btn btn-secondary" data-action="qc-start" data-fk="qc-start" ${f.busy ? 'disabled' : ''}>${icon('quick-connect')}Use Quick Connect</button><span class="caption">Approve from a device that's already signed in. Needs the server address.</span></div>` : ''}
      </div>`;
    }
    if (f.step === 'plex-wait') {
      return `${flowHead('Connect to Plex')}<div class="form"><div class="waiting" role="status"><span class="spinner light" aria-hidden="true"></span><div class="od-field od-fill"><span>Waiting for you to sign in to Plex in your browser…</span><span class="caption">Perfuse continues automatically when you're done.</span></div></div>
        <div class="form-actions"><button type="button" class="btn btn-secondary" data-action="flow-cancel-wait" data-fk="flow-cancel">${icon('close')}Cancel</button><button type="button" class="btn btn-text" data-action="plex-reopen">Open the sign-in page again</button></div></div>`;
    }
    if (f.step === 'qc-wait') {
      return `${flowHead('Jellyfin Quick Connect')}<div class="form">
        <p class="muted">On a device that's already signed in to Jellyfin, open your user settings, choose Quick Connect and enter this code:</p>
        <p class="qc-code" aria-label="Code 4 7 2 9 1 3">472 913</p>
        <div class="waiting" role="status"><span class="spinner light" aria-hidden="true"></span><span class="od-fill">Waiting for approval from ${esc(hostOf(f.values.address))}…</span></div>
        <div class="form-actions"><button type="button" class="btn btn-secondary" data-action="flow-cancel-wait" data-fk="flow-cancel">${icon('close')}Cancel</button></div></div>`;
    }
    if (f.step === 'plex-pick') {
      const count = f.picks.filter((p) => p.checked && !p.added).length;
      return `${flowHead('Choose Plex servers')}<div class="form">
        <p class="muted">Signed in as sam@example.com. Choose the servers to add to Perfuse.</p>
        <div class="pick-list">${f.picks.map((p, i) => `<label class="pick"><input type="checkbox" data-pick="${i}" data-fk="pick-${i}" ${p.checked ? 'checked' : ''} ${p.added ? 'disabled' : ''}><span class="od-field"><span class="od-truncate">${esc(p.name)}</span><span class="s od-truncate">${esc(p.address)}</span></span><span class="tag">${p.added ? 'Already added' : `${p.owned ? 'Your server' : 'Shared with you'} · ${p.libraries} libraries`}</span></label>`).join('')}</div>
        <div class="form-actions"><button type="button" class="btn btn-primary" data-action="plex-add" data-fk="plex-add" ${count && !f.busy ? '' : 'disabled'}>${f.busy ? '<span class="spinner" aria-hidden="true"></span>Adding…' : `${icon('add')}${count ? `Add ${count} ${count === 1 ? 'server' : 'servers'}` : 'Choose a server'}`}</button></div></div>`;
    }
    if (f.step === 'success') {
      const rs = f.results.map(serverById).filter(Boolean);
      const one = rs.length === 1 ? rs[0] : null;
      const title = one ? `${one.name} is connected` : `${rs.length} servers connected`;
      return `<div class="success-head"><span class="ok">${icon('check')}</span><h2 id="${ctx === 'dialog' ? 'add-dialog-title' : 'flow-title'}" style="font-size:22px;flex:1">${esc(title)}</h2>${ctx === 'dialog' ? `<button type="button" class="icon-btn" data-action="flow-close" aria-label="Close">${icon('close')}</button>` : ''}</div>
        <div class="form">
          ${one ? `<div class="field"><label for="f-result-name">Name in Perfuse</label><input id="f-result-name" type="text" data-result-name="${one.id}" data-fk="f-result-name" value="${esc(one.name)}" maxlength="40" aria-describedby="f-result-help"><p class="help" id="f-result-help">Shown in the sidebar as ${esc(KINDS[one.kind].label)} · name.</p></div>` : ''}
          <div class="added-list">${rs.map((s) => `<div class="added">${kindIcon(s)}<div class="od-field od-fill"><span class="od-truncate">${esc(s.name)}</span><span class="s od-truncate">Found ${s.libraries.length} ${s.libraries.length === 1 ? 'library' : 'libraries'}: ${esc(s.libraries.map((l) => l.name).join(', '))}</span></div><span class="ok">${icon('check')}</span></div>`).join('')}</div>
          <div class="form-actions"><button type="button" class="btn btn-primary" data-action="flow-finish" data-fk="flow-finish">${ctx === 'dialog' ? 'Done' : 'Continue to Perfuse'}</button><button type="button" class="btn btn-secondary" data-action="flow-another" data-fk="flow-another">${icon('add')}Add another server</button></div>
        </div>`;
    }
    return '';
  }
  const hostOf = (a) => { try { return new URL(a).host; } catch { return a; } };

  function renderFlow() {
    if (!state.flow) return;
    const html = flowHTML();
    if (state.flowCtx === 'dialog') $('#add-dialog-body').innerHTML = html;
    else { const c = $('#onboard-card-body'); if (c) c.innerHTML = html; }
  }
  function viewOnboarding() {
    return `<div class="onboard-card"><div class="onboard-logo" aria-label="Perfuse"><svg viewBox="84 56 124 140" aria-hidden="true"><g clip-path="url(#pf-mark)"><rect x="60" y="54" width="160" height="41.33" fill="#8CC8FF"/><rect x="60" y="107.33" width="160" height="41.33" fill="#3D9BFF"/><rect x="60" y="160.67" width="160" height="41.33" fill="#1F6CCB"/></g></svg><span aria-hidden="true">PERFUSE</span></div><div id="onboard-card-body"></div></div>`;
  }

  function serverNameFrom(kind, address) {
    const host = hostOf(address).split(':')[0];
    if (/^\d+\.\d+\.\d+\.\d+$/.test(host) || !host) return `${KINDS[kind].label} server`;
    const first = host.split('.')[0];
    return first.charAt(0).toUpperCase() + first.slice(1);
  }
  function uniqueName(name) {
    let n = name, i = 2;
    while (state.servers.some((s) => s.name.toLowerCase() === n.toLowerCase())) n = `${name} ${i++}`;
    return n;
  }
  function createServer(kind, address, account, nameHint, libCount) {
    const seed = findSeedByAddress(address);
    let s;
    if (seed && !serverById(seed.id)) {
      s = clone(seed);
      s.status = 'online'; s.lastSeen = null;
    } else {
      const defaults = {
        plex: [['Movies', 'movies'], ['TV Shows', 'shows'], ['Documentaries', 'movies'], ['Kids', 'shows']],
        jellyfin: [['Movies', 'movies'], ['Shows', 'shows']],
        audiobookshelf: [['Audiobooks', 'audiobooks']],
      }[kind].slice(0, libCount || undefined);
      const id = `srv-${kind}-${Date.now().toString(36)}${Math.floor(Math.random() * 1e4)}`;
      s = {
        id, kind, name: uniqueName(nameHint || serverNameFrom(kind, address)), address, account,
        latencyMs: 20 + Math.floor(Math.random() * 30), status: 'online', lastSeen: null, collapsed: false,
        libraries: defaults.map(([name, type], i) => ({ id: `${id}-lib${i}`, name, type, visible: true })),
      };
    }
    state.servers.push(s);
    return s;
  }

  function flowConnect() {
    const f = state.flow;
    const key = f.kind === 'plex' ? 'plex-manual' : f.kind;
    const req = REQUIRED[key];
    const errs = [];
    for (const name of req.concat(f.kind === 'jellyfin' ? [] : [])) {
      const msg = validate(name);
      f.errors[name] = msg || undefined;
      if (msg) errs.push({ name, msg });
    }
    if (errs.length) {
      f.banner = errs.length > 1 ? { type: 'error', title: `Fix ${errs.length} fields to continue`, fields: errs } : null;
      renderFlow();
      const first = document.getElementById(`f-${errs[0].name}`);
      if (first) first.focus();
      return;
    }
    f.banner = null;
    f.busy = true;
    renderFlow();
    announce('Connecting…');
    f.timer = setTimeout(() => {
      f.busy = false;
      const addr = f.values.address.trim();
      if (/offline|unreachable/i.test(addr)) {
        f.banner = { type: 'error', title: `Couldn't reach ${hostOf(addr)}`, text: 'Check the address and that the server is running, then try again.' };
        renderFlow();
        $('#flow-banner')?.focus();
        return;
      }
      if ((f.values.password || f.values.token) === 'wrong') {
        f.banner = { type: 'error', title: f.kind === 'plex' ? 'That token was rejected' : 'Username or password is incorrect', text: 'Check your details and try again.' };
        renderFlow();
        $('#flow-banner')?.focus();
        return;
      }
      const s = createServer(f.kind, addr, f.kind === 'plex' ? 'Plex token' : f.values.username.trim());
      f.results = [s.id];
      f.step = 'success';
      renderAll();
      focusLater('#f-result-name');
      announce(`${s.name} connected`);
    }, 1300);
  }

  // ── main render ──────────────────────────────────────────
  let liveChapterIdx = null;
  function renderAll() {
    const ae = document.activeElement;
    const fk = ae && ae.dataset ? ae.dataset.fk : null;
    const content = $('#content');
    const railScroll = {};
    $$('.rail', content).forEach((r) => { railScroll[r.id] = r.scrollLeft; });
    const top = content.scrollTop;

    if (!state.servers.length) state.onboarding = true;
    const onboarding = state.onboarding;
    $('#body').classList.toggle('no-sidebar', onboarding);
    $('#sidebar').hidden = onboarding;
    content.classList.toggle('onboard', onboarding);
    $('#back-btn').disabled = onboarding || !state.history.length;

    if (onboarding) {
      if (!state.flow || state.flowCtx !== 'onboard') { state.flow = newFlow(); state.flowCtx = 'onboard'; }
      content.innerHTML = viewOnboarding();
      renderFlow();
      $('#dock-slot').innerHTML = '';
    } else {
      renderSidebar();
      const v = state.view;
      let html;
      switch (v.name) {
        case 'server': html = viewServer(serverById(v.serverId)); break;
        case 'library': { const s = serverById(v.serverId); html = viewLibrary(s, libOf(s, v.libraryId)); break; }
        case 'video': html = viewVideo(videoById(v.id)); break;
        case 'book': html = viewBook(bookById(v.id)); break;
        case 'player': html = viewPlayer(); break;
        case 'settings': html = viewSettings(); break;
        default: html = viewHome();
      }
      content.innerHTML = html;
      $('#dock-slot').innerHTML = renderDock();
      content.scrollTop = top;
      $$('.rail', content).forEach((r) => { if (railScroll[r.id]) r.scrollLeft = railScroll[r.id]; });
    }
    const b = bookById(state.player.bookId);
    liveChapterIdx = b ? chapterAt(b, b.position) : null;
    if (fk) {
      const el = document.querySelector(`[data-fk="${CSS.escape(fk)}"]`);
      if (el && el !== document.activeElement && !el.disabled) el.focus({ preventScroll: true });
    }
  }

  function updateLive() {
    const b = bookById(state.player.bookId);
    if (!b) return;
    const ci = chapterAt(b, b.position);
    if (ci !== liveChapterIdx) { renderAll(); return; }
    const c = b.chapters[ci];
    const total = bookTotal(b);
    const inCh = b.position - c.start;
    const set = (k, t) => $$(`[data-live="${k}"]`).forEach((el) => { el.textContent = t; });
    set('chap-elapsed', fmtClock(inCh));
    set('chap-remaining', `−${fmtClock(c.duration - inCh)}`);
    set('book-elapsed', fmtClock(b.position));
    set('book-left', bookLeftText(b));
    set('dock-l', fmtClock(inCh));
    set('dock-r', `−${fmtClock(c.duration - inCh)}`);
    set('sleep', sleepLabel());
    $$('[data-live-w="chap"]').forEach((el) => { el.style.width = `${((inCh / c.duration) * 100).toFixed(2)}%`; });
    $$('[data-live-w="book"]').forEach((el) => { el.style.width = `${((b.position / total) * 100).toFixed(2)}%`; });
    const seek = $('#seek');
    if (seek && document.activeElement !== seek) {
      seek.value = Math.floor(inCh);
      seek.style.setProperty('--pct', `${((inCh / c.duration) * 100).toFixed(2)}%`);
      seek.setAttribute('aria-valuetext', `${fmtClock(inCh)} of ${fmtClock(c.duration)}`);
    }
  }

  // ── navigation ───────────────────────────────────────────
  function go(view) {
    state.history.push({ view: state.view, scroll: $('#content').scrollTop });
    state.view = view;
    state.menu = null;
    state.status.open = false;
    renderAll();
    $('#content').scrollTop = 0;
    focusHeading();
  }
  function back() {
    const prev = state.history.pop();
    if (!prev) return;
    state.view = prev.view;
    state.menu = null;
    renderAll();
    $('#content').scrollTop = prev.scroll;
    focusHeading();
  }
  function focusHeading() {
    const h = $('#page-title');
    if (h) { h.setAttribute('tabindex', '-1'); h.focus({ preventScroll: true }); }
    else $('#content').focus({ preventScroll: true });
  }
  function focusLater(sel) { requestAnimationFrame(() => { const el = typeof sel === 'string' ? $(sel) : sel; if (el) el.focus(); }); }

  // ── playback ─────────────────────────────────────────────
  function startBook(id, fromStart) {
    const b = bookById(id);
    if (!b) return;
    if (fromStart || b.finished) { b.position = 0; b.finished = false; }
    state.player.bookId = id;
    state.player.playing = true;
    renderAll();
    announce(`Playing ${b.title}`);
  }
  function seekTo(b, pos) {
    b.position = Math.max(0, Math.min(bookTotal(b) - 1, pos));
    b.finished = false;
  }
  function tick() {
    const p = state.player;
    const b = bookById(p.bookId);
    if (!b || !p.playing) return;
    const before = chapterAt(b, b.position);
    b.position = Math.min(bookTotal(b), b.position + p.speed);
    if (p.sleep && p.sleep.mode === 'minutes') {
      p.sleep.remaining -= 1;
      if (p.sleep.remaining <= 0) { p.playing = false; p.sleep = null; toast('Sleep timer ended. Playback paused.'); renderAll(); return; }
    }
    const after = chapterAt(b, b.position);
    if (after !== before && p.sleep && p.sleep.mode === 'chapter') {
      p.playing = false; p.sleep = null; b.position = b.chapters[after].start;
      toast('Paused at the end of the chapter.');
      renderAll();
      return;
    }
    if (b.position >= bookTotal(b)) { p.playing = false; b.finished = true; b.position = 0; toast(`Finished ${b.title}`); renderAll(); return; }
    updateLive();
  }
  setInterval(tick, 1000);

  // ── server status actions ───────────────────────────────
  function retry(ids) {
    const targets = ids.map(serverById).filter(Boolean);
    if (!targets.length) return;
    targets.forEach((s) => { s.status = 'checking'; });
    renderAll();
    announce('Checking servers…');
    setTimeout(() => {
      targets.forEach((s) => { s.status = 'online'; s.lastSeen = null; });
      renderAll();
      toast(targets.length === 1 ? `Connected to ${targets[0].name}` : 'All servers connected');
    }, 1200);
  }
  function setScenario(n) {
    state.onboarding = false;
    if (n === 0) {
      state.servers = [];
      state.player = { bookId: null, playing: false, speed: 1, sleep: null, tab: 'chapters' };
      state.history = [];
      state.view = { name: 'home' };
      state.flow = null;
      renderAll();
      return;
    }
    if (!state.servers.length) state.servers = clone(window.SEED_SERVERS);
    state.servers.forEach((s, i) => {
      const off = n === 3 || (n === 2 && i === 1);
      s.status = off ? 'offline' : 'online';
      s.lastSeen = off ? '4 min ago' : null;
    });
    renderAll();
  }

  // ── misc UI ──────────────────────────────────────────────
  function announce(msg) { const l = $('#live'); l.textContent = ''; requestAnimationFrame(() => { l.textContent = msg; }); }
  let toastTimer = null;
  function toast(msg) {
    $('#toast-slot').innerHTML = `<div class="toast" role="status">${esc(msg)}</div>`;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { $('#toast-slot').innerHTML = ''; }, 2800);
  }
  function closeMenu(returnFocus) {
    if (!state.menu) return;
    const fk = state.menu.triggerFk;
    state.menu = null;
    renderAll();
    if (returnFocus && fk) focusLater(`[data-fk="${fk}"]`);
  }
  function openAddDialog() {
    state.flow = newFlow();
    state.flowCtx = 'dialog';
    renderFlow();
    const d = $('#add-dialog');
    d.showModal();
    focusLater('[data-fk="kind-plex"]');
  }
  function closeAddDialog() {
    const f = state.flow;
    if (f && f.timer) clearTimeout(f.timer);
    const d = $('#add-dialog');
    if (d.open) d.close();
  }

  // ── events ───────────────────────────────────────────────
  document.addEventListener('click', (e) => {
    const el = e.target.closest('[data-action]');
    if (state.menu && !e.target.closest('.menu') && !(el && el.dataset.action === 'menu')) closeMenu(false);
    if (state.status.open && !e.target.closest('#sidebar-foot')) { state.status.open = false; state.status.pinned = false; renderSidebar(); }
    if (!el) return;
    const a = el.dataset.action;
    const d = el.dataset;
    const p = state.player;
    const curBook = bookById(p.bookId);
    switch (a) {
      case 'back': back(); break;
      case 'go-home': go({ name: 'home' }); break;
      case 'go-settings': go({ name: 'settings' }); break;
      case 'go-server': go({ name: 'server', serverId: d.server }); break;
      case 'go-library': go({ name: 'library', serverId: d.server, libraryId: d.lib }); break;
      case 'open-video': go({ name: 'video', id: d.id }); break;
      case 'open-book': go({ name: 'book', id: d.id }); break;
      case 'open-player':
        if (d.tab) p.tab = d.tab;
        if (state.view.name !== 'player') go({ name: 'player' });
        break;
      case 'toggle-sort': state.sortAlpha[d.server] = !state.sortAlpha[d.server]; renderAll(); break;
      case 'toggle-collapse': { const s = serverById(d.server); s.collapsed = !s.collapsed; renderAll(); break; }
      case 'toggle-status':
        state.status.pinned = !state.status.open || !state.status.pinned;
        state.status.open = state.status.pinned;
        renderSidebar();
        if (state.status.open) $('#status-btn')?.focus();
        break;
      case 'retry-server': retry([d.server]); break;
      case 'retry-all': retry(state.servers.filter((s) => s.status !== 'online').map((s) => s.id).concat(state.servers.every(isOnline) ? state.servers.map((s) => s.id) : [])); break;
      case 'rail': { const r = $(`#rail-${d.rail}`); if (r) r.scrollBy({ left: Number(d.dir) * r.clientWidth * 0.8 }); break; }
      case 'video-play': { const v = videoById(d.id); toast(`Starting ${v.title}`); break; }
      case 'book-play': startBook(d.id, false); break;
      case 'book-start': startBook(d.id, true); break;
      case 'toggle-play': if (curBook) { p.playing = !p.playing; renderAll(); announce(p.playing ? 'Playing' : 'Paused'); } break;
      case 'skip': if (curBook) { seekTo(curBook, curBook.position + Number(d.value)); if (chapterAt(curBook, curBook.position) !== liveChapterIdx) renderAll(); else updateLive(); } break;
      case 'prev-chapter': if (curBook) {
        const ci = chapterAt(curBook, curBook.position);
        const c = curBook.chapters[ci];
        seekTo(curBook, curBook.position - c.start > 3 || ci === 0 ? c.start : curBook.chapters[ci - 1].start);
        renderAll();
      } break;
      case 'next-chapter': if (curBook) {
        const ci = chapterAt(curBook, curBook.position);
        if (ci < curBook.chapters.length - 1) { seekTo(curBook, curBook.chapters[ci + 1].start); renderAll(); }
      } break;
      case 'play-chapter': {
        const b = bookById(d.id);
        seekTo(b, b.chapters[Number(d.index)].start);
        p.bookId = b.id; p.playing = true;
        renderAll();
        announce(`Playing ${b.chapters[Number(d.index)].title}`);
        break;
      }
      case 'stop-player': p.playing = false; p.bookId = null; p.sleep = null; renderAll(); focusHeading(); break;
      case 'menu': {
        const same = state.menu && state.menu.type === d.menu && state.menu.where === d.where;
        state.menu = same ? null : { type: d.menu, where: d.where, triggerFk: el.dataset.fk };
        renderAll();
        if (!same) focusLater('#menu [aria-checked="true"]');
        break;
      }
      case 'set-speed': p.speed = Number(d.value); closeMenu(true); announce(`Speed ${fmtSpeed(p.speed)}`); break;
      case 'set-sleep': {
        const v = d.value;
        p.sleep = v === 'off' ? null : v === 'chapter' ? { mode: 'chapter' } : { mode: 'minutes', minutes: Number(v), remaining: Number(v) * 60 };
        closeMenu(true);
        toast(p.sleep ? (p.sleep.mode === 'chapter' ? 'Pausing at the end of this chapter' : `Sleep timer set for ${v} minutes`) : 'Sleep timer off');
        break;
      }
      case 'bm-add': if (curBook) {
        const c = curBook.chapters[chapterAt(curBook, curBook.position)];
        curBook.bookmarks.push({ id: `bm-${Date.now()}`, at: Math.floor(curBook.position), note: `${c.title} · ${fmtClock(curBook.position - c.start)}` });
        p.tab = 'bookmarks';
        renderAll();
        toast(`Bookmark added at ${fmtClock(curBook.position)}`);
      } break;
      case 'bm-jump': { const b = bookById(d.id); const m = b.bookmarks.find((x) => x.id === d.bm); seekTo(b, m.at); p.bookId = b.id; p.playing = true; renderAll(); break; }
      case 'bm-delete': {
        const b = bookById(d.id);
        b.bookmarks = b.bookmarks.filter((x) => x.id !== d.bm);
        renderAll();
        focusLater(state.view.name === 'player' ? '[data-fk="tab-bookmarks"]' : '[data-fk="bk-primary"]');
        toast('Bookmark deleted');
        break;
      }
      case 'tab': p.tab = d.value; renderAll(); break;
      case 'add-server': openAddDialog(); break;
      case 'toggle-row': state.expanded.has(d.server) ? state.expanded.delete(d.server) : state.expanded.add(d.server); renderAll(); break;
      case 'rename': state.renaming = d.server; renderAll(); focusLater(`#rename-${d.server}`); break;
      case 'remove-server': {
        const s = serverById(d.server);
        state.pendingRemove = s.id;
        $('#confirm-title').textContent = `Remove ${s.name}?`;
        $('#confirm-text').textContent = `Perfuse will sign out of this ${KINDS[s.kind].label} server and forget its saved sign-in. Your progress stays on the server.`;
        $('#confirm-dialog').showModal();
        focusLater('[data-action="confirm-cancel"]');
        break;
      }
      case 'confirm-cancel': $('#confirm-dialog').close(); break;
      case 'confirm-ok': {
        const s = serverById(state.pendingRemove);
        state.pendingRemove = null;
        $('#confirm-dialog').close();
        if (!s) break;
        state.servers = state.servers.filter((x) => x.id !== s.id);
        if (curBook && curBook.serverId === s.id) { p.bookId = null; p.playing = false; }
        state.history = state.history.filter((h) => h.view.serverId !== s.id);
        renderAll();
        toast(`Removed ${s.name}`);
        focusLater(state.servers.length ? '[data-fk="add-server"]' : '[data-fk="kind-plex"]');
        break;
      }
      // add-server flow
      case 'flow-kind': state.flow = newFlow(d.kind); renderFlow(); focusLater(d.kind === 'plex' ? '[data-fk="plex-signin"]' : '#f-address'); break;
      case 'flow-back': if (state.flow.timer) clearTimeout(state.flow.timer); state.flow = newFlow(); renderFlow(); focusLater(`[data-fk="kind-${d.kind || 'plex'}"]`); break;
      case 'flow-close': closeAddDialog(); break;
      case 'flow-connect': flowConnect(); break;
      case 'focus-field': $(`#f-${d.field}`)?.focus(); break;
      case 'plex-signin': {
        const f = state.flow;
        f.step = 'plex-wait'; f.banner = null;
        renderFlow();
        focusLater('[data-fk="flow-cancel"]');
        f.timer = setTimeout(() => {
          if (state.flow !== f || f.step !== 'plex-wait') return;
          f.step = 'plex-pick';
          f.picks = window.PLEX_DISCOVERED.map((x) => {
            const added = state.servers.some((s) => normAddr(s.address) === normAddr(x.address));
            return { ...x, added, checked: x.owned && !added };
          });
          renderFlow();
          focusLater('.pick input:not(:disabled)');
        }, 2200);
        break;
      }
      case 'plex-reopen': toast('Opened the Plex sign-in page in your browser'); break;
      case 'flow-cancel-wait': { const f = state.flow; clearTimeout(f.timer); f.step = 'form'; renderFlow(); focusLater(f.kind === 'plex' ? '[data-fk="plex-signin"]' : '[data-fk="qc-start"]'); break; }
      case 'plex-add': {
        const f = state.flow;
        f.busy = true; renderFlow();
        f.timer = setTimeout(() => {
          f.busy = false;
          f.results = f.picks.filter((x) => x.checked && !x.added).map((x) => createServer('plex', x.address, 'sam@example.com', x.name, x.libraries).id);
          f.step = 'success';
          renderAll();
          focusLater(f.results.length === 1 ? '#f-result-name' : '[data-fk="flow-finish"]');
        }, 1000);
        break;
      }
      case 'qc-start': {
        const f = state.flow;
        const msg = validate('address');
        setFieldError('address', msg);
        if (msg) { $('#f-address')?.focus(); break; }
        f.step = 'qc-wait'; f.banner = null;
        renderFlow();
        focusLater('[data-fk="flow-cancel"]');
        f.timer = setTimeout(() => {
          if (state.flow !== f || f.step !== 'qc-wait') return;
          const s = createServer('jellyfin', f.values.address.trim(), 'Quick Connect');
          f.results = [s.id]; f.step = 'success';
          renderAll();
          focusLater('#f-result-name');
        }, 4000);
        break;
      }
      case 'flow-another':
        state.flow = newFlow();
        renderFlow();
        focusLater('[data-fk="kind-plex"]');
        break;
      case 'flow-finish':
        if (state.flowCtx === 'dialog') { closeAddDialog(); renderAll(); }
        else { state.onboarding = false; state.flow = null; state.view = { name: 'home' }; state.history = []; renderAll(); focusHeading(); }
        break;
      default: break;
    }
  });

  // Status popover: opens on hover/focus, pins on click.
  let hoverTimer = null;
  document.addEventListener('mouseover', (e) => {
    if (!e.target.closest('#status-btn') || state.status.open) return;
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => { state.status.open = true; renderSidebar(); }, 250);
  });
  document.addEventListener('mouseout', (e) => {
    const foot = $('#sidebar-foot');
    if (!foot || !foot.contains(e.target)) return;
    if (e.relatedTarget && foot.contains(e.relatedTarget)) return;
    clearTimeout(hoverTimer);
    if (state.status.open && !state.status.pinned) { state.status.open = false; renderSidebar(); }
  });
  document.addEventListener('focusin', (e) => {
    if (e.target.id === 'status-btn' && !state.status.open && e.target.matches(':focus-visible')) {
      state.status.open = true;
      renderSidebar();
      $('#status-btn')?.focus();
    }
  });

  document.addEventListener('input', (e) => {
    const t = e.target;
    if (t.dataset.field && state.flow) {
      state.flow.values[t.dataset.field] = t.value;
      if (state.flow.errors[t.dataset.field] && !validate(t.dataset.field)) setFieldError(t.dataset.field, '');
    }
    if (t.dataset.resultName) {
      const s = serverById(t.dataset.resultName);
      if (s && t.value.trim()) s.name = t.value.trim();
    }
    if (t.id === 'seek') {
      const b = bookById(state.player.bookId);
      const c = b.chapters[chapterAt(b, b.position)];
      seekTo(b, c.start + Number(t.value));
      t.style.setProperty('--pct', `${((Number(t.value) / c.duration) * 100).toFixed(2)}%`);
      t.setAttribute('aria-valuetext', `${fmtClock(Number(t.value))} of ${fmtClock(c.duration)}`);
      updateLive();
      $$('[data-live="chap-elapsed"]').forEach((el) => { el.textContent = fmtClock(Number(t.value)); });
    }
  });
  document.addEventListener('change', (e) => {
    const t = e.target;
    if (t.dataset.fieldCheck && state.flow) state.flow.values[t.dataset.fieldCheck] = t.checked;
    if (t.dataset.pick !== undefined && state.flow) { state.flow.picks[Number(t.dataset.pick)].checked = t.checked; renderFlow(); }
    if (t.dataset.libtoggle) {
      const s = serverById(t.dataset.server);
      const l = libOf(s, t.dataset.libtoggle);
      l.visible = t.checked;
      renderAll();
      announce(`${l.name} ${l.visible ? 'shown' : 'hidden'}`);
    }
  });
  document.addEventListener('toggle', (e) => {
    if (e.target.matches && e.target.matches('details[data-manual]') && state.flow) state.flow.manualOpen = e.target.open;
  }, true);
  document.addEventListener('focusout', (e) => {
    const t = e.target;
    if (t.dataset && t.dataset.field && state.flow && t.value.trim() !== '') setFieldError(t.dataset.field, validate(t.dataset.field));
    if (t.dataset && t.dataset.rename) commitRename(t, true);
  });

  function commitRename(input, save) {
    const s = serverById(input.dataset.rename);
    if (state.renaming !== s.id) return;
    const v = input.value.trim();
    if (save && v) s.name = v;
    state.renaming = null;
    renderAll();
    focusLater(`[data-fk="row-rename-${s.id}"]`);
  }

  document.addEventListener('keydown', (e) => {
    const t = e.target;
    if (e.ctrlKey && e.shiftKey && /^Digit[0-3]$/.test(e.code)) { e.preventDefault(); setScenario(Number(e.code.slice(5))); return; }
    if (t.dataset && t.dataset.rename) {
      if (e.key === 'Enter') { e.preventDefault(); commitRename(t, true); }
      if (e.key === 'Escape') { e.preventDefault(); commitRename(t, false); }
      return;
    }
    if (t.dataset && t.dataset.field && e.key === 'Enter' && state.flow && state.flow.step === 'form') { e.preventDefault(); flowConnect(); return; }
    if (t.dataset && t.dataset.handle && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      moveServer(t.dataset.handle, e.key === 'ArrowUp' ? -1 : 1);
      return;
    }
    if (e.key === 'Escape') {
      if (state.menu) { e.preventDefault(); closeMenu(true); return; }
      if (state.status.open) { state.status.open = false; state.status.pinned = false; renderSidebar(); $('#status-btn')?.focus(); }
    }
    if (state.menu && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      const items = $$('#menu button');
      const i = items.indexOf(document.activeElement);
      if (items.length) { e.preventDefault(); items[(i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length].focus(); }
    }
  });

  function moveServer(id, dir) {
    const i = state.servers.findIndex((s) => s.id === id);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= state.servers.length) return;
    const [s] = state.servers.splice(i, 1);
    state.servers.splice(j, 0, s);
    renderAll();
    focusLater(`[data-fk="handle-${id}"]`);
    announce(`${s.name} moved to position ${j + 1} of ${state.servers.length}`);
  }

  // drag and drop reorder (Settings)
  document.addEventListener('dragstart', (e) => {
    const row = e.target.closest && e.target.closest('.srow');
    if (!row) return;
    state.dragId = row.dataset.row;
    row.classList.add('dragging');
    e.dataTransfer.effectAllowed = 'move';
    try { e.dataTransfer.setData('text/plain', state.dragId); } catch { /* some hosts block dataTransfer */ }
  });
  document.addEventListener('dragover', (e) => {
    const row = e.target.closest && e.target.closest('.srow');
    if (!row || !state.dragId) return;
    e.preventDefault();
    $$('.srow.drag-over').forEach((r) => r !== row && r.classList.remove('drag-over'));
    if (row.dataset.row !== state.dragId) row.classList.add('drag-over');
  });
  document.addEventListener('drop', (e) => {
    const row = e.target.closest && e.target.closest('.srow');
    if (!row || !state.dragId) return;
    e.preventDefault();
    const from = state.servers.findIndex((s) => s.id === state.dragId);
    const to = state.servers.findIndex((s) => s.id === row.dataset.row);
    if (from > -1 && to > -1 && from !== to) {
      const [s] = state.servers.splice(from, 1);
      state.servers.splice(to, 0, s);
      announce(`${s.name} moved to position ${to + 1} of ${state.servers.length}`);
    }
    state.dragId = null;
    renderAll();
  });
  document.addEventListener('dragend', () => { state.dragId = null; $$('.srow').forEach((r) => r.classList.remove('dragging', 'drag-over')); });

  $('#add-dialog').addEventListener('close', () => {
    const f = state.flow;
    if (f && f.timer) clearTimeout(f.timer);
    state.flow = null;
    state.flowCtx = null;
    renderAll();
    focusLater($('[data-fk="add-server"]') || $('#page-title') || $('#content'));
  });
  $('#confirm-dialog').addEventListener('close', () => {
    if (state.pendingRemove) { const fk = `row-remove-${state.pendingRemove}`; state.pendingRemove = null; focusLater(`[data-fk="${fk}"]`); }
  });

  // window chrome + initial render
  $('#back-btn').innerHTML = icon('back');
  $('#win-ctrls').innerHTML = ['pin', 'minimize', 'maximize', 'close'].map((n) => `<span class="win-ctrl">${icon(n)}</span>`).join('');
  try { const ps = new URLSearchParams(location.search).get('state'); if (ps) setScenario(Number(ps)); } catch { /* sandboxed */ }
  renderAll();
})();
