# Social preview

The 1280 × 640 PNG is rendered from [social-preview.svg](social-preview.svg).
The SVG retains the existing repository card's purple palette, border and
type hierarchy. It contains text and geometry only.

The card paraphrases README.md and the review boundaries in AGENTS.md at
commit 1d7ef1b1028704581d97019b4cf386d4d2d55b9d.
Checked 11 September 2026. No package version, accounting rule or source-review
date changes in this card.

Render with Node.js and Sharp 0.35.4 available in a development environment,
using Windows Segoe UI and Consolas fonts:

```bash
node -e "require('sharp')('.github/social-preview.svg').png().toFile('.github/social-preview.png')"
```

Open the PNG and check its text and 1280 × 640 dimensions before use. Font
substitution on another platform can change its appearance.

GitHub stores its social preview separately from repository files. After
review, upload [social-preview.png](social-preview.png) under Settings,
General, Social preview. Merging this file does not update that setting.
