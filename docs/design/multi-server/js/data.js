// Sample data for the multi-server prototype. Titles follow the synthetic demo
// fixture (tools/demo_server.py); audiobooks are fictional placeholders.
// Shape mirrors the suggested Rust model in handoff/HANDOFF.md.

window.SERVER_KINDS = {
  plex: { label: 'Plex', icon: 'server-plex', signIn: 'browser' },
  jellyfin: { label: 'Jellyfin', icon: 'server-jellyfin', signIn: 'password' },
  audiobookshelf: { label: 'Audiobookshelf', icon: 'server-audiobookshelf', signIn: 'password' },
};

window.LIBRARY_ICONS = { movies: 'movies', shows: 'tv-shows', audiobooks: 'audiobooks', podcasts: 'podcasts' };

window.SEED_SERVERS = [
  {
    id: 'srv-plex-basement', kind: 'plex', name: 'Basement', address: 'https://192.168.1.20:32400',
    account: 'sam@example.com', latencyMs: 12, status: 'online', lastSeen: null, collapsed: false,
    libraries: [
      { id: 'lib-movies', name: 'Movies', type: 'movies', count: 412, visible: true },
      { id: 'lib-shows', name: 'TV Shows', type: 'shows', count: 58, visible: true },
      { id: 'lib-lectures', name: 'Lectures', type: 'shows', count: 14, visible: true },
    ],
  },
  {
    id: 'srv-jf-media', kind: 'jellyfin', name: 'Media', address: 'http://media.lan:8096',
    account: 'sam', latencyMs: 31, status: 'online', lastSeen: null, collapsed: false,
    libraries: [
      { id: 'lib-jf-films', name: 'Films', type: 'movies', count: 236, visible: true },
      { id: 'lib-jf-anime', name: 'Anime', type: 'shows', count: 22, visible: true },
    ],
  },
  {
    id: 'srv-abs-library', kind: 'audiobookshelf', name: 'Library', address: 'https://books.home.arpa',
    account: 'sam', latencyMs: 18, status: 'online', lastSeen: null, collapsed: false,
    libraries: [
      { id: 'lib-abs-books', name: 'Audiobooks', type: 'audiobooks', count: 187, visible: true },
    ],
  },
];

// Placeholder art palettes, matching the generated fixture artwork.
window.PALETTES = {
  teal: ['#2b5f72', '#0f2630'], navy: ['#2d4a7c', '#0d1a33'], rust: ['#8a3a2a', '#1c0d0a'],
  pine: ['#2a6a5a', '#0b1f1a'], slate: ['#3b4d63', '#10161f'], plum: ['#5b3a5e', '#1a0f1c'],
  ochre: ['#8a6a2a', '#21190a'], moss: ['#4a5f2f', '#141a0c'],
};

window.SEED_VIDEOS = [
  { id: 'v-quiet', serverId: 'srv-plex-basement', libraryId: 'lib-movies', title: 'The Quiet Earth', year: 2020, minutes: 120, progress: 0.17, palette: 'teal', badge: '4K', recent: 1 },
  { id: 'v-blue', serverId: 'srv-plex-basement', libraryId: 'lib-movies', title: 'Beyond the Blue', year: 2021, minutes: 114, progress: 0.27, palette: 'navy', badge: '4K', recent: 2 },
  { id: 'v-after', serverId: 'srv-jf-media', libraryId: 'lib-jf-films', title: 'After Hours', year: 2022, minutes: 118, progress: 0.42, palette: 'rust', badge: '4K', recent: 3 },
  { id: 'v-north', serverId: 'srv-jf-media', libraryId: 'lib-jf-films', title: 'Northern Lights', year: 2023, minutes: 104, progress: 0, palette: 'pine', badge: '4K', recent: 4 },
  { id: 'v-place', serverId: 'srv-plex-basement', libraryId: 'lib-movies', title: 'A Place Between the Stars', year: 2024, minutes: 131, progress: 0, palette: 'slate', badge: '4K', recent: 5 },
  { id: 'v-train', serverId: 'srv-plex-basement', libraryId: 'lib-movies', title: 'The Last Train', year: 2025, minutes: 97, progress: 0, palette: 'navy', badge: '4K', recent: 6, watched: true },
  { id: 'v-harbour', serverId: 'srv-plex-basement', libraryId: 'lib-shows', title: 'Harbour Lights', year: 2022, minutes: 48, progress: 0.61, palette: 'ochre', badge: 'S2 · E4', recent: 7, episode: 'S2 · E4 — The Crossing' },
  { id: 'v-lect', serverId: 'srv-plex-basement', libraryId: 'lib-lectures', title: 'Systems Thinking 101', year: 2019, minutes: 52, progress: 0, palette: 'moss', badge: 'Lecture 3', recent: 8 },
  { id: 'v-lantern', serverId: 'srv-jf-media', libraryId: 'lib-jf-anime', title: 'Lantern Festival', year: 2021, minutes: 24, progress: 0.35, palette: 'plum', badge: 'S1 · E7', recent: 9, episode: 'S1 · E7 — Paper Boats' },
];

function chapters(list) {
  let start = 0;
  return list.map(([title, sec], i) => {
    const c = { index: i, title, start, duration: sec };
    start += sec;
    return c;
  });
}

window.SEED_BOOKS = [
  {
    id: 'b-tidewater', serverId: 'srv-abs-library', libraryId: 'lib-abs-books', title: 'The Tidewater Archive',
    author: 'Mara Ellison', narrator: 'Jonah Reyes', series: 'Tidewater', seriesIndex: 2, year: 2023, palette: 'teal', recent: 1,
    description: 'An archivist on a flooded coast finds a ledger that records events before they happen, and has to decide which pages to keep.',
    chapters: chapters([
      ['Opening Credits', 42], ['Prologue: High Water', 1260], ['1. The Ledger', 2210], ['2. Salt and Paper', 1985], ['3. The Ferryman', 2420],
      ['4. A Map of Nowhere', 2105], ['5. Lowtide', 1890], ['6. The Second Hand', 2330], ['7. Weatherglass', 2015], ['8. The Index Room', 2650],
      ['9. Brackish', 1780], ['10. What the River Kept', 2290], ['11. The Long Pier', 2045], ['12. Ink in the Water', 2380], ['Epilogue', 1120], ['End Credits', 38],
    ]),
    position: 21130, // inside chapter 9
    bookmarks: [{ id: 'bm-1', at: 9120, note: 'The ledger entry for March' }],
  },
  {
    id: 'b-orchard', serverId: 'srv-abs-library', libraryId: 'lib-abs-books', title: 'Orchard at the Edge of the Map',
    author: 'Priya Natarajan', narrator: 'Priya Natarajan', series: null, year: 2024, palette: 'moss', recent: 2,
    description: 'A memoir of planting a heritage orchard on reclaimed farmland, told across twelve seasons.',
    chapters: chapters([
      ['Introduction', 640], ['Winter: Bare Root', 2480], ['Spring: Grafting', 2730], ['Summer: Thinning', 2390], ['Autumn: First Fruit', 2860],
      ['Winter Again', 2210], ['Afterword', 720],
    ]),
    position: 4100,
    bookmarks: [],
  },
  {
    id: 'b-signal', serverId: 'srv-abs-library', libraryId: 'lib-abs-books', title: 'Signal and Static',
    author: 'Dev Okafor', narrator: 'Hannah Lowe', series: 'Relay', seriesIndex: 1, year: 2022, palette: 'navy', recent: 3,
    description: 'A night-shift radio operator picks up a broadcast from a station that closed thirty years ago.',
    chapters: chapters([
      ['Part One', 30], ['Chapter 1', 1840], ['Chapter 2', 1720], ['Chapter 3', 2010], ['Chapter 4', 1655], ['Part Two', 25],
      ['Chapter 5', 1930], ['Chapter 6', 2105], ['Chapter 7', 1870], ['Chapter 8', 2240],
    ]),
    position: 0,
    bookmarks: [],
  },
  {
    id: 'b-stone', serverId: 'srv-abs-library', libraryId: 'lib-abs-books', title: 'A Short History of Stone',
    author: 'Ilse Varga', narrator: 'Tom Ashby', series: null, year: 2021, palette: 'slate', recent: 4,
    description: 'Geology for the curious: how mountains, quarries and cathedrals share the same story.',
    chapters: chapters([['Introduction', 900], ['Deep Time', 2600], ['Fire', 2450], ['Pressure', 2380], ['Water', 2290], ['Hands', 2720], ['Coda', 800]]),
    position: 0,
    bookmarks: [],
    finished: true,
  },
  {
    id: 'b-ember', serverId: 'srv-abs-library', libraryId: 'lib-abs-books', title: 'Ember Road',
    author: 'Mara Ellison', narrator: 'Jonah Reyes', series: 'Tidewater', seriesIndex: 1, year: 2021, palette: 'rust', recent: 5,
    description: 'The first Tidewater novel: a courier carries a sealed letter across a burning province.',
    chapters: chapters([['Opening Credits', 40], ['1. The Letter', 2100], ['2. Smoke', 1980], ['3. Crossroads', 2260], ['4. The Toll', 2045], ['5. Embers', 2390], ['End Credits', 35]]),
    position: 0,
    bookmarks: [],
    finished: true,
  },
  {
    id: 'b-quiet', serverId: 'srv-abs-library', libraryId: 'lib-abs-books', title: 'The Quiet Hours',
    author: 'Noor Haddad', narrator: 'Leah Castell', series: null, year: 2025, palette: 'plum', recent: 6,
    description: 'Essays on attention, written between midnight and dawn.',
    chapters: chapters([['Foreword', 520], ['Midnight', 1650], ['One', 1540], ['Two', 1720], ['Three', 1480], ['Four', 1610], ['Dawn', 1390]]),
    position: 2950,
    bookmarks: [],
  },
];

// Servers a Plex account exposes after browser sign-in (add-server flow).
window.PLEX_DISCOVERED = [
  { name: 'Basement', address: 'https://192.168.1.20:32400', owned: true, libraries: 3 },
  { name: 'Cabin', address: 'https://cabin.plex.direct:32400', owned: true, libraries: 2 },
  { name: "Alex's Plex", address: 'https://10-0-0-8.alex.plex.direct:32400', owned: false, libraries: 4 },
];
