# Brand assets

The ironstate mark is **`→(Fe)`** — a transition arrow flowing into a parenthesized iron tile. It reads as "a transition into iron state," i.e. *ironstate*:

- **`→`** the arrow is a *state transition* — the most universal symbol of a state machine, the move from one state to the next.
- **`( )`** the parentheses frame the state as a value, the way transition/function notation groups a term.
- **`Fe` / `26`** iron, atomic number 26 — solid, durable, verified.

The palette is two-tone with a clear hierarchy: **amber is the machine/notation** (the arrow and parentheses, with a darker amber outline) and **slate is the iron** (the element tile, a subtle top-down gradient with a beveled edge). The atomic number is amber too, tying the tile to the mark.

| Color | Hex | Role |
|-------|-----|------|
| Amber | `#f59e0b` | arrow + parentheses fill, the `26` accent |
| Dark amber | `#b45309` | outline on the arrow + parentheses |
| Slate | `#1f2933` → `#141b22` | tile fill (gradient), wordmark text on light |
| Slate edge | `#3b4a55` | tile border |
| Off-white | `#e8edf2` | the `Fe` symbol |
| Slate gray | `#9fb3c4` | social subtitle, wordmark text on dark |
| Mid gray | `#64748b` | neutral wordmark text (legible on light or dark) |

## Files

| File | Size | Where it's used |
|------|------|-----------------|
| `logo.svg` / `logo.png` | 256² / 512² | docs.rs sidebar logo (`html_logo_url`) |
| `favicon-32.png` | 32² | docs.rs favicon (`html_favicon_url`) |
| `wordmark.svg` / `wordmark.png` | 760×220 | README hero — neutral mid-gray, the `<picture>` fallback |
| `wordmark-light.svg` / `wordmark-light.png` | 760×220 | wordmark for light backgrounds (dark slate text) |
| `wordmark-dark.svg` / `wordmark-dark.png` | 760×220 | wordmark for dark backgrounds (slate-gray text) |
| `social-preview.svg` / `social-preview.png` | 1280×640 | GitHub social preview (upload via repo Settings) |

The PNGs are what's embedded (Markdown and docs.rs reference them); the SVGs are the scalable masters.

The README hero swaps the wordmark by color scheme with a `<picture>`: a `prefers-color-scheme: dark`/`light` source picks the crisp variant, and the neutral `wordmark.png` is the fallback. GitHub honors the swap by theme. docs.rs embeds the crate README (`#![doc = include_str!("../README.md")]`) but keys the media query off the **OS** color scheme, not its own light/dark/ayu switch — so the neutral fallback is what keeps it legible there regardless of the in-page theme. The sidebar logo is separate (`logo.png` via `html_logo_url`) and theme-independent.

## Regenerating

The assets are generated from one source so the logo, wordmark, and social card stay in sync. Rendering uses [`resvg`](https://github.com/linebender/resvg) (`cargo install resvg`), which honors the SVG viewBox faithfully — macOS `qlmanage` auto-fits the visible ink and renders transparent marks off-center, so it is not used here.

```sh
cargo install resvg     # once
python3 assets/brand.py # rewrites the .svg sources and re-renders the .png files
```

Edit proportions, colors, or layout in `brand.py` (a single `mark()` builds the shared symbol); never hand-edit the generated SVG paths.
