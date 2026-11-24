# Source ↔ preview sync
(authored by agents unless marked 🧑)

problem: SyncTeX maps PDF points to source lines only (column is always -1), and its line is sometimes a neighbor

approach: SyncTeX for the rough spot, then letter matching for the exact character
- `server/match.js`, shared by server and viewer
    - `letters(text, tex)`: lowercase letters/digits only, NFKD (ligatures, accents), with index map
        - `tex`: skip control-sequence names and `%` comments
        - so hyphenation, spacing, ligatures, `\emph{...}` braces do not matter
    - `bestSplit(hay, before, after, prefer)`: position maximizing matched suffix of `before` + prefix of `after`; ties → nearest `prefer`

inverse (click → editor), `server/sync.js` `inverse`
- viewer sends page, PDF point, and 40 letters before/after the clicked caret (`caretPositionFromPoint` in the pdf.js text layer)
- `synctex edit` → file, line
- match in ±6 lines; if < 80% of context letters match, match the whole file
- editor jumps via LSP `window/showDocument` with a selection

forward (cursor → viewer), `forward`
- `synctex view` → page and box; server adds 40 source letters around the cursor
- viewer matches them against that page's text layer, preferring spans overlapping the box
- draws a blinking caret; falls back to highlighting the box
- triggered by the `llp/cursor` notification (Neovim plugin, on cursor move) or command `llp.forwardSearch`

limits
- text without letters (pure symbols, figures) maps to the SyncTeX line start
- math: rendered glyphs rarely match source letters; falls back to the line
- uses the PDF that was clicked; if its dir was GC'd, the latest done PDF
