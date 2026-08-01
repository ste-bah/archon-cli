#!/usr/bin/env node
// Merge a plugin's hooks/settings.snippet.json into an .archon/settings.json.
// Used by install.sh / install.ps1. Idempotent: a hook entry is skipped if an
// entry with the same command already exists under the same event.
//
// Usage: node merge-hooks.js <settings.json> <snippet.json>

const fs = require('fs');

const [settingsPath, snippetPath] = process.argv.slice(2);
if (!settingsPath || !snippetPath) {
  console.error('usage: node merge-hooks.js <settings.json> <snippet.json>');
  process.exit(1);
}

const snippet = JSON.parse(fs.readFileSync(snippetPath, 'utf8'));
let settings = {};
if (fs.existsSync(settingsPath)) {
  const raw = fs.readFileSync(settingsPath, 'utf8').trim();
  settings = raw ? JSON.parse(raw) : {};
}

if (!settings.hooks) settings.hooks = {};

const commandsOf = (entry) =>
  (entry.hooks || []).map((h) => h.command).filter(Boolean);

let added = 0;
for (const [event, entries] of Object.entries(snippet.hooks || {})) {
  if (!settings.hooks[event]) settings.hooks[event] = [];
  const existing = new Set(
    settings.hooks[event].flatMap(commandsOf)
  );
  for (const entry of entries) {
    const cmds = commandsOf(entry);
    if (cmds.length > 0 && cmds.every((c) => existing.has(c))) continue;
    settings.hooks[event].push(entry);
    cmds.forEach((c) => existing.add(c));
    added += 1;
  }
}

fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2) + '\n');
console.log(`merged ${added} hook entr${added === 1 ? 'y' : 'ies'} into ${settingsPath}`);
