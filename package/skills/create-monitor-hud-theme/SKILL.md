---
name: create-monitor-hud-theme
description: Design, validate, preview, and package shareable Agent Monitor HUD themes. Use when a user wants a new HUD skin, a richer visual preset, a JSON or .cmhud-theme file, a .cmhud-theme.zip pack with local artwork, or help turning a visual reference into a safe lightweight theme.
---

# Create Monitor HUD Theme

Create polished themes that remain legible at HUD scale, install by drag-and-drop, and never execute code.

## Workflow

1. Read [references/theme-format.md](references/theme-format.md) completely before writing a theme.
2. Inspect any visual reference and the current `themes/*.json` files when a Agent Monitor HUD source tree is available.
3. Choose the smallest suitable format:
   - Use JSON or `.cmhud-theme` for colors, gradients, typography, geometry, density, status colors, automatic reminder appearance, and the separately styled proactive-Codex-notice appearance.
   - Use `.cmhud-theme.zip` only when the design needs a local PNG or JPG background asset.
4. Establish the visual brief from the request: mood, light/dark preference, contrast, information density, motion strength, and any accessibility constraint. Make reasonable choices when the user already gave enough direction.
5. Build the theme with one clear visual idea. Preserve status semantics and keep the live numbers more legible than decoration.
6. Validate every enum, color, numeric range, relative asset path, and package limit against the reference.
7. When the project runtime is available, render at least one summary preview and one list or split preview. Inspect both at native scale, not only enlarged.
8. Deliver the shareable theme file and state that it can be dropped onto **Settings > General > Theme workshop**. Do not install it unless the user asks.

## Design Rules

- Optimize for a small always-on-top monitor, not a wallpaper or dashboard.
- Keep primary metrics readable over every surface. Reduce image opacity before adding text shadows.
- Use one accent family plus semantic status colors. Completion, abort, and error must remain distinguishable.
- Prefer gradients, geometry, density, and restrained motion over excessive decoration.
- Treat strong pulse or focus animations as attention tools, not ambient animation.
- Test the status dot and icon-like details at 8-16 physical pixels.
- Keep background assets static, local, and small. Never reference remote URLs.

## Safety and Packaging

- Never include executable files, scripts, fonts, DLLs, links, or network resources in a theme pack.
- A theme may style proactive Codex notices but must never enable them, grant expressive permission, or alter notice safety limits.
- A ZIP pack may contain only root `theme.json` plus PNG/JPG files under `assets/`.
- Keep the full pack at or below 5 MB, each asset at or below 3 MB, and the package at or below 16 files.
- Use lowercase hyphenated theme IDs. Do not use absolute paths or `..` path segments.
- Do not modify Codex, Codex logs, databases, user prompts, or GitHub while creating a theme.

## Output Contract

Return:

- the finished `.json`, `.cmhud-theme`, or `.cmhud-theme.zip` file;
- a one-sentence visual description;
- the tested surfaces and any limitation;
- the drag-and-drop installation location.

If the current Monitor HUD version does not support a requested effect, explain the gap and produce the closest safe theme instead of patching application behavior without permission.
