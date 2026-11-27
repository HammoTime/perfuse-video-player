// The HTML reference uses the same logical widths as the native shelves.
(() => {
  'use strict';
  const gap = 16;
  const content = document.getElementById('content');
  const allRails = () => Array.from(content.querySelectorAll('.rail'));
  const firstOnPage = rail => {
    const page = Math.round(rail.scrollLeft / (rail.clientWidth + gap));
    return rail.children[page]?.querySelector('.card');
  };
  const positions = () => new Map(allRails().map(rail => [rail.id, firstOnPage(rail)?.dataset.fk]));

  function updateArrows(rail) {
    const buttons = rail.parentElement.querySelector('.rail-nav');
    buttons.hidden = rail.childElementCount <= 1;
    buttons.firstElementChild.disabled = rail.scrollLeft < 1;
    buttons.lastElementChild.disabled = rail.scrollWidth - rail.clientWidth - rail.scrollLeft < 1;
  }

  function layout(rail, savedKey) {
    const cards = Array.from(rail.querySelectorAll('.card'));
    if (!cards.length) { updateArrows(rail); return; }
    const kind = cards[0].classList;
    const [min, max] = kind.contains('square') ? [176, 224] : kind.contains('poster') ? [160, 210] : [240, 320];
    const columns = Math.max(1, Math.floor((rail.clientWidth + gap) / (min + gap)));
    const width = Math.min(max, (rail.clientWidth - (columns - 1) * gap) / columns);
    const focused = cards.find(card => card === document.activeElement);
    const anchor = focused || cards.find(card => card.dataset.fk === savedKey) || firstOnPage(rail);
    // Use the section's position at rest so ordinary vertical scrolling cannot change its row count.
    let rows = 1;
    if (rail.id === 'rail-cw' && cards.length > columns) {
      const top = rail.getBoundingClientRect().top + content.scrollTop;
      const remaining = content.getBoundingClientRect().bottom - top;
      const videoRow = width * 9 / 16 + 71;
      const next = document.getElementById('rail-cl');
      const bookColumns = Math.max(1, Math.floor((rail.clientWidth + gap) / (176 + gap)));
      const bookWidth = Math.min(224, (rail.clientWidth - (bookColumns - 1) * gap) / bookColumns);
      const nextShelfHeight = next ? 48 + 44 + bookWidth + 71 : 0;
      if (remaining >= 2 * videoRow + gap + nextShelfHeight) rows = 2;
    }
    rail.style.setProperty('--shelf-width', width + 'px');
    rail.style.setProperty('--shelf-columns', columns);
    const signature = columns + 'x' + rows;
    if (rail.dataset.layout !== signature) {
      rail.dataset.layout = signature;
      const pageSize = columns * rows;
      const pages = document.createDocumentFragment();
      for (let i = 0; i < cards.length; i += pageSize) {
        const page = document.createElement('div');
        page.className = 'shelf-page';
        page.append(...cards.slice(i, i + pageSize));
        pages.append(page);
      }
      rail.replaceChildren(pages);
      focused?.focus({ preventScroll: true });
    }
    if (anchor) rail.scrollTo({ left: anchor.parentElement.offsetLeft - rail.firstElementChild.offsetLeft, behavior: 'instant' });
    updateArrows(rail);
  }

  // Preview the independent completion of local fixture covers, including one slow cover.
  const ready = new Set();
  const loading = new Map();
  const completed = new Set();
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');
  let frame = 0;
  const keyOf = card => card.dataset.id + '/' + (card.classList.contains('poster') ? 'poster' : 'preview');
  function reveal() {
    frame = 0;
    let index = 0;
    for (const card of content.querySelectorAll('.card.art-pending')) {
      if (!completed.has(keyOf(card))) continue;
      card.classList.remove('art-pending');
      const bounds = card.getBoundingClientRect();
      const rail = card.closest('.rail')?.getBoundingClientRect() || content.getBoundingClientRect();
      if (!reduced.matches && bounds.right > rail.left && bounds.left < rail.right &&
          bounds.top < content.getBoundingClientRect().bottom && bounds.bottom > content.getBoundingClientRect().top) {
        card.querySelector('.art-svg')?.animate([{opacity: 0}, {opacity: 1}],
          { duration: 83, delay: Math.min(index++ * 16, 48), fill: 'backwards' });
      }
    }
    completed.clear();
  }
  const near = new IntersectionObserver(entries => {
    for (const { target: card, isIntersecting } of entries) {
      if (!isIntersecting || !card.isConnected) continue;
      near.unobserve(card);
      const key = keyOf(card);
      if (ready.has(key)) { card.classList.remove('art-pending'); continue; }
      if (loading.has(key)) continue;
      loading.set(key, setTimeout(() => {
        loading.delete(key);
        ready.add(key);
        completed.add(key);
        if (!frame) frame = requestAnimationFrame(reveal);
      }, card.dataset.id === 'v-blue' ? 1600 : 180));
    }
  }, { rootMargin: '96px', scrollMargin: '96px' });

  function watchArt() {
    if (!window.SHELF_PREVIEW) return;
    near.disconnect();
    for (const card of content.querySelectorAll('.card')) {
      const pending = !ready.has(keyOf(card));
      card.classList.toggle('art-pending', pending);
      if (pending) near.observe(card);
    }
  }
  function replay() {
    loading.forEach(clearTimeout);
    loading.clear();
    completed.clear();
    ready.clear();
    watchArt();
  }
  function restore(keys) {
    for (const rail of allRails()) {
      layout(rail, keys?.get(rail.id));
      rail.onscroll = () => updateArrows(rail);
    }
  }
  function attach(keys) { restore(keys); watchArt(); }
  function page(rail, direction) {
    rail.scrollBy({ left: direction * (rail.clientWidth + gap) });
  }
  new ResizeObserver(() => restore()).observe(content);
  content.addEventListener('keydown', event => {
    const card = event.target.closest('.rail .card');
    if (!card) return;
    const rail = card.closest('.rail');
    const columns = Number(rail.style.getPropertyValue('--shelf-columns'));
    const delta = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columns, ArrowDown: columns }[event.key];
    if (!delta) return;
    const cards = Array.from(rail.querySelectorAll('.card'));
    const next = cards[cards.indexOf(card) + delta];
    if (next) { event.preventDefault(); next.focus(); }
  });
  window.shelves = { attach, page, positions, restore, replay };
})();
