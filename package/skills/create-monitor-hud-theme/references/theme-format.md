# Agent Monitor HUD theme format

## Choose a container

- `.json` or `.cmhud-theme`: one declarative UTF-8 JSON document. It cannot reference image assets.
- `.cmhud-theme.zip`: a ZIP with `theme.json` at the root and optional PNG/JPG files below `assets/`.

The importer does not execute scripts and does not load network URLs.

## Base document

```json
{
  "format": "codex-monitor-hud-theme",
  "formatVersion": 1,
  "id": "signal-glass",
  "order": 500,
  "author": "Theme author",
  "description": "Deep glass with a restrained cyan signal.",
  "names": {
    "zh-CN": "Signal Glass（信号玻璃）",
    "en": "Signal Glass",
    "symbols": "Signal Glass"
  },
  "settings": {}
}
```

`id` must match `^[a-z0-9][a-z0-9-]{1,47}$`. `names` and `settings` are required. Metadata fields such as `format`, `formatVersion`, `author`, and `description` are recommended but optional.

## Supported settings

### Core appearance

```json
{
  "background": "#EE071A2B",
  "foreground": "#FFF5FBFF",
  "muted": "#FF9CC7DE",
  "accent": "#FF32ADE6",
  "border": "#304DD4FF",
  "cornerRadius": 24,
  "opacity": 0.97,
  "fontSize": 14.0,
  "layout": "chips",
  "separator": "dot",
  "transparencyMode": "layered",
  "showStatusDot": true,
  "animateUpdates": true
}
```

Colors accept WPF-compatible hex ARGB. Prefer `#AARRGGBB`. Layout: `chips`, `compact`, `inline`, `outline`, `cards`, or `stacked`. Transparency: `uniform`, `layered`, or `focus`.

### Surface and typography

```json
{
  "themeStyle": {
    "surface": "gradient",
    "gradientStart": "#F0071A2B",
    "gradientEnd": "#E90B4962",
    "gradientAngle": 24,
    "backgroundImage": "",
    "imageOpacity": 0.28,
    "imageStretch": "uniformToFill",
    "shadow": "soft",
    "borderWidth": 1.2,
    "statusDotSize": 8.5,
    "fontFamily": "Segoe UI Variable Text, Microsoft YaHei UI"
  }
}
```

- `surface`: `solid`, `gradient`, or `image`.
- `gradientAngle`: any degree value; normalized to 0-359.
- `backgroundImage`: only for ZIP packs, relative to `theme.json`, normally `assets/background.png`.
- `imageStretch`: `uniform`, `uniformToFill`, `fill`, or `none`.
- `shadow`: `none`, `soft`, or `deep`.
- `imageOpacity`: 0.05-1.0; keep detailed art near 0.15-0.35.
- `borderWidth`: 0-4.
- `statusDotSize`: 5-18.
- `fontFamily`: use an installed system font and include sensible fallbacks separated by commas.

### Multi-task visual density

```json
{
  "multiTask": {
    "listStyle": "rail",
    "listDensity": "compact",
    "nameMode": "always"
  }
}
```

List styles: `rows`, `cards`, `rail`. Density: `compact`, `balanced`, `relaxed`. Name mode: `hover`, `always`, `hidden`.

### Reminder appearance

```json
{
  "attention": {
    "summaryMode": "halo",
    "listMode": "flow",
    "taskBubbleMode": "flow",
    "dotEnabled": true,
    "dotPattern": "heartbeat",
    "dotBrightness": "balanced",
    "dotSpeed": "normal",
    "dotBreathing": true
  }
}
```

Surface modes: `off`, `halo`, `breathe`, `flow`, `focus`. Dot pattern: `soft`, `heartbeat`, `beacon`. Brightness: `subtle`, `balanced`, `bright`. Speed: `slow`, `normal`, `fast`.

Themes may style reminders but may not enable or disable reminder triggers such as completion, abort, or quiet-time inference.

Proactive Codex notices have a separate visual block so a theme can make model-authored messages unmistakable without changing user permission:

```json
{
  "agentNotification": {
    "mode": "focus",
    "color": "#FF7C3AED",
    "glowPreset": "custom",
    "intensity": "balanced"
  }
}
```

`mode`: `halo`, `breathe`, `flow`, `focus`. `glowPreset`: `violet`, `aqua`, `amber`, `custom`. `intensity`: `subtle`, `balanced`, `strong`. A theme may style these four fields only. It cannot enable proactive notices, grant expressive permission, change message limits, or define executable animation code.

### Status palette

```json
{
  "statusColors": {
    "active": "#FF30D158",
    "listening": "#FF32ADE6",
    "idle": "#FF8E8E93",
    "paused": "#FFFF9F0A",
    "error": "#FFFF453A",
    "completed": "#FF30D158",
    "aborted": "#FFFF453A"
  }
}
```

Supply all seven colors when creating a deliberate palette. Do not make error and normal listening visually identical.

## ZIP layout

```text
signal-glass.cmhud-theme.zip
├── theme.json
└── assets/
    └── background.png
```

Only `theme.json` and `.png`, `.jpg`, or `.jpeg` files below `assets/` are accepted. The archive may contain at most 16 files, at most 5 MB total uncompressed data, and at most 3 MB per entry. Absolute paths, drive letters, `..`, and other file types are rejected.

## Full gradient example

```json
{
  "format": "codex-monitor-hud-theme",
  "formatVersion": 1,
  "id": "signal-glass",
  "names": {"zh-CN":"Signal Glass（信号玻璃）","en":"Signal Glass","symbols":"Signal Glass"},
  "settings": {
    "background":"#EF061827",
    "foreground":"#FFF4FCFF",
    "muted":"#FF92B6C8",
    "accent":"#FF22D3EE",
    "border":"#5524D3FF",
    "cornerRadius":24,
    "opacity":0.97,
    "transparencyMode":"layered",
    "themeStyle":{"surface":"gradient","gradientStart":"#F0061827","gradientEnd":"#E9084E68","gradientAngle":28,"shadow":"deep","borderWidth":1.2,"statusDotSize":8.5,"fontFamily":"Segoe UI Variable Text, Microsoft YaHei UI"},
    "multiTask":{"listStyle":"rail","listDensity":"compact","nameMode":"always"},
    "attention":{"summaryMode":"halo","listMode":"flow","taskBubbleMode":"flow","dotEnabled":true,"dotPattern":"heartbeat","dotBrightness":"balanced","dotSpeed":"normal","dotBreathing":true}
  }
}
```
