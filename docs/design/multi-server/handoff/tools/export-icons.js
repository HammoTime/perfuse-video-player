// Writes every NEW glyph from js/icons.js to handoff/icons/<file> in the
// assets/icons format, plus a preview sheet. Run: node handoff/tools/export-icons.js
const fs = require('fs');
const path = require('path');
const root = path.resolve(__dirname, '..', '..');
const sandbox = { window: {} };
new Function('window', fs.readFileSync(path.join(root, 'js', 'icons.js'), 'utf8'))(sandbox.window);
const icons = sandbox.window.PERFUSE_ICONS;
const out = path.join(root, 'handoff', 'icons');
fs.mkdirSync(out, { recursive: true });
const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/"/g, '&quot;');
const rows = [];
for (const [name, def] of Object.entries(icons)) {
  if (!def.isNew) continue;
  const label = esc(def.label);
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" role="img" aria-label="${label}"><title>${label}</title>${def.body}</svg>\n`;
  fs.writeFileSync(path.join(out, def.file), svg);
  rows.push(`<tr><td class="g">${svg}</td><td><code>${def.file}</code></td><td>${label}</td><td>${esc(def.use || '')}</td></tr>`);
}
fs.writeFileSync(path.join(out, 'preview.html'), `<!doctype html><meta charset="utf-8"><title>Perfuse — new icons (multi-server)</title>
<style>body{background:#0d0d10;color:#ebebf5;font:15px/1.5 'Segoe UI',sans-serif;padding:32px}table{border-collapse:collapse}td,th{padding:10px 16px;border-bottom:1px solid #2a2a2e;text-align:left;vertical-align:middle}th{color:#98989f;font-weight:400}.g svg{width:24px;height:24px;color:#ebebf5}code{color:#8cc8ff}</style>
<h1 style="font-weight:300">New icons for multi-server Perfuse</h1><p style="color:#98989f">Same contract as assets/icons: 24×24, 1.5 stroke, round caps/joins, currentColor (rendered white then tinted by src/icons.rs).</p>
<table><tr><th>Glyph</th><th>File</th><th>Label</th><th>Used for</th></tr>${rows.join('\n')}</table>\n`);
console.log(`${rows.length} icons written`);
