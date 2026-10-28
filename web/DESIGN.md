# Raindrop Web Design Direction

## 1. Visual theme and atmosphere

Raindrop is a quiet reading workspace: warm paper surfaces, ink-blue controls, and crisp information density. Smartisan-inspired tactile edges meet Apple's restrained hierarchy and immediate feedback, without leather textures, oversized glass panels, or decorative 3D motion. Navigation stays utilitarian and sans-serif; article titles and reading content use a serif-led stack so the product feels like a reader rather than an admin template.

## 2. Color palette and roles

| Token | Value | Role |
| --- | --- | --- |
| Canvas | `oklch(0.975 0.008 85)` | Warm application background |
| Surface | `oklch(0.995 0.004 85)` | Forms and elevated reading surfaces |
| Ink | `oklch(0.23 0.025 255)` | Primary text |
| Muted ink | `oklch(0.49 0.018 255)` | Secondary text and metadata |
| Accent | `oklch(0.42 0.09 250)` | Primary actions and active navigation |
| Border | `oklch(0.89 0.012 85)` | Hairline separation |
| Danger | `oklch(0.54 0.17 27)` | Errors and destructive actions only |

Dark mode uses a warm-black canvas with small surface lightness steps. It never uses pure black or full white.

## 3. Typography rules

- Controls: `-apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC", "Noto Sans SC", sans-serif`.
- Reading: `Charter, "Source Han Serif SC", "Noto Serif CJK SC", "Songti SC", Georgia, serif`.
- Code and counters: `ui-monospace, "SFMono-Regular", Consolas, monospace` with tabular numerals.
- Chinese body copy uses line-height `1.75`; English reading copy uses `1.65`. Negative tracking is limited to Latin display headings.

## 4. Component styling

ASTRYX is the component source. Buttons, inputs, banners, navigation, dialogs, and loading feedback keep ASTRYX structure and accessibility. Raindrop changes theme tokens and domain layout only. The radius scale is `6px`, `10px`, `14px`, and pill; buttons use the `10px` tier and content surfaces use `14px`. A restrained top highlight, subtle control sheen, and contact shadow make buttons feel raised; inputs use a recessed shadow. Disabled controls do not receive a new press effect.

## 5. Layout principles

Setup and login use a calm two-region composition on desktop: product context beside a focused form. Ready state uses one `AppShell`; future reader views use one responsive `Layout`. Sections are mostly cardless, with surface elevation reserved for forms and reading content.

The reader keeps CommaFeed's efficiency grammar rather than its page shape: an unread-first source tree, a stable entry queue, fast keyboard traversal, snapshot-based bulk read actions, and non-disruptive new-entry notices. Raindrop modernizes this as a three-pane desktop workspace, a two-region medium layout, and single-task mobile routes with deep links and restored scroll anchors. AI summaries, translations, and plugin artifacts are secondary reader sidecars; original feed content remains the default and never waits on them.

## 6. Depth and elevation

Material roles are explicit: `--surface-navigation` is the warm structural layer, `--surface-chrome` frames controls, and the reading surface remains quiet and opaque. Depth comes from lightness steps, fine edges, and a shared elevation scale. `--surface-control-sheen` is a local material treatment on controls and selected items, never a decorative page gradient.

Floating menus and dialogs use the existing `--surface-overlay-background` and `--surface-overlay-filter` tokens, with a separate `--surface-backdrop-filter` for modal scrims. Keep translucency out of article text and ordinary pane backgrounds. Both the user's independent blur/shadow switches and system reduced-transparency preference remain authoritative. Increased contrast makes overlays solid and strengthens boundaries.

## 7. Do and do not

- Do use ASTRYX props and tokens before business CSS.
- Do keep errors next to their fields and retain user input after failures.
- Do keep one accent color per screen.
- Do provide skip-to-content and visible focus behavior.
- Do not create local generic Button, Dialog, Selector, or input wrappers.
- Do not compress desktop side panels into mobile.
- Do not animate keyboard-driven navigation or routine list changes.
- Do not scale a positioned control in a way that replaces its positioning transform. Reading-dock, quick-read, and selection actions retain their own spatial behavior and use material feedback instead.
- Do not automatically merge newly arrived entries into the active queue or move the current selection.
- Do not make summary, translation, or plugin output a prerequisite for reading the original article.

## 8. Responsive behavior

- `>= 1100px`: full desktop workspace.
- `720px-1099px`: contextual two-region layout.
- `< 720px`: one task per route with `AppShell + MobileNav`.
- Mobile uses `100dvh`, safe-area padding, 44px minimum targets, and content-owned scrolling at 390x844 and 360x800.

## 9. Motion and implementation prompt

Pressable controls use the shared 120ms `scale(0.97)` response. Setup steps and mobile detail entrances use an interruptible 180ms opacity/transform transition; frequent desktop article selection and queue updates stay instant. Authentication forms use opacity only so immediately interactive controls keep stable hit targets. Centered dialogs materialize at `scale(0.97)` over 240ms and return along the same path over 180ms, without waiting to accept input. Trigger-anchored menus retain ASTRYX's positioning and focus management.

`InterfaceMotion` records pointer, keyboard, and virtual-click input without React state or a wrapper element. Keyboard-triggered entrances and presses have no animation. Reduced motion removes spatial movement while preserving short opacity feedback. Loading uses a responsive reader silhouette and a single 1000ms opacity-breathing status dot; reduced motion leaves the dot static. Article skeletons follow title, metadata, and paragraph structure instead of showing a solid block. Never add artificial loading or transition delays.

Implementation prompt: compose screens from ASTRYX 0.1.6, use canvas `oklch(0.975 0.008 85)`, ink `oklch(0.23 0.025 255)`, accent `oklch(0.42 0.09 250)`, button radius `10px`, surface radius `14px`, system sans controls, Charter plus explicit CJK serif reading text, and no custom generic controls.
