# Application identity marks

These small vector paths identify monitored applications. Brand artwork and trademarks remain the property of their respective owners; the project MIT license covers its original code, not third-party brand marks.

- `openai.path`: the official OpenAI regular knot path from the locally installed Codex application asset `openai_logo_regular-ceb75a1754fa.svg` (24-unit viewbox). Rendered white for Codex desktop identity and in the existing colour for account quota.
- `claude.path`: the official Claude flower from https://claude.ai/favicon.svg (248-unit viewbox).
- `clawd.path`: the official Clawd silhouette from https://claude.ai/images/clawd.svg (66 by 52 viewbox). The two square eyes are transparent cutouts over a dark badge, keeping the small monochrome identity recognisable.
- `deepseek.path`: the official DeepSeek whale from https://fe-static.deepseek.com/chat/favicon.svg, referenced by the DeepSeek chat page (50-unit viewbox). Rendered in the original `#4D6BFE` colour for Harness identity and balance groups.
- `vscode.path`: the exact even-odd silhouette mask from the locally installed VS Code `resources/app/out/media/code-icon.svg` (256-unit viewbox). Rendered in the existing HUD blue, without an added outline or the original large-icon shadows.

`F1` selects the original nonzero fill rule; `F0` selects even-odd filling for the Clawd eye cutouts and VS Code silhouette. The native renderer scales and fills these paths without adding an outline. No application bundle, fonts, screenshots or animation files are included.
