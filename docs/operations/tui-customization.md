# TUI customization

archon-cli's terminal UI is built on `ratatui` and supports themes, vim mode, and custom keybindings.

## Themes

23 themes total: 16 MBTI personality themes + 7 utility themes + 1 `auto` indirection.

```bash
archon --list-themes
archon --theme dracula              # Set at startup
```

In the TUI:
```
/theme                              # Cycle through themes
/theme intj                         # Switch to specific theme
/color                              # Change prompt bar accent only
```

### MBTI themes

| Theme | Type | Palette |
|---|---|---|
| `intj` | Architect | Midnight blue, cold cyan |
| `intp` | Logician | Deep navy, icy slate |
| `entj` | Commander | Steel blue, amber gold |
| `entp` | Debater | Electric teal, bright magenta |
| `infj` | Advocate | Deep violet, rose |
| `infp` | Mediator | Soft indigo, warm pink |
| `enfj` | Protagonist | Warm violet, golden amber |
| `enfp` | Campaigner | Vibrant purple, bright rose |
| `istj` | Logistician | Forest green, warm grey |
| `isfj` | Defender | Sage green, warm cream |
| `estj` | Executive | Deep navy, earth brown |
| `esfj` | Consul | Warm teal, soft gold |
| `istp` | Virtuoso | Slate grey, sharp orange |
| `isfp` | Adventurer | Warm beige, terracotta |
| `estp` | Entrepreneur | Bold red, vivid yellow |
| `esfp` | Entertainer | Coral, energetic yellow |

Auto-selection: setting `[personality] type = "INTJ"` defaults the theme to `intj`.

### Utility themes

| Theme | Description |
|---|---|
| `dark` | Classic dark terminal |
| `light` | Light background |
| `ocean` | Deep blue ocean |
| `fire` | Red/orange fire |
| `forest` | Natural greens |
| `mono` | Monochrome grey |
| `daltonized` | Colorblind-friendly palette |
| `auto` | System dark/light detection (currently resolves to `dark`) |

## Vim mode

Toggle via:
```toml
[tui]
vim_mode = true
```

Or in the TUI: `/vim`.

### Bindings (modal input)

**Normal mode** (Esc to enter):

| Key | Action |
|---|---|
| `i` | Insert mode at cursor |
| `a` | Insert mode after cursor |
| `I` | Insert at start of line |
| `A` | Insert at end of line |
| `o` | Open new line below |
| `O` | Open new line above |
| `h` `j` `k` `l` | Move cursor |
| `w` `b` `e` | Word motion |
| `0` `$` | Start / end of line |
| `gg` `G` | Top / bottom of buffer |
| `dd` | Delete line |
| `yy` | Yank line |
| `p` `P` | Paste after / before |
| `u` | Undo |
| `Ctrl-r` | Redo |
| `:` | Command mode |

**Insert mode** behaves like default text input. Esc returns to Normal mode.

## Status line

Configure via `/statusline` skill. Common segments:
- Current model
- Token count / cost
- Effort level
- Permission mode
- Active session name
- Active git branch

## Keybindings

```
/keybindings                        # Show full reference
```

Default shortcuts:
- `Ctrl-C` — cancel current operation
- `Ctrl-D` — exit
- `Ctrl-L` — clear screen
- `Ctrl-R` — reverse history search
- `Ctrl-O` — toggle output verbosity
- `Esc` — vim normal mode (when vim_mode = true)
- `Tab` — slash command autocomplete
- `Up` / `Down` — history navigation

## See also

- [Configuration](../reference/config.md) — `[tui]` and `[personality]` sections
- [Slash commands](../reference/slash-commands.md) — `/theme`, `/vim`, `/statusline`
