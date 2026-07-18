# Raindrop Web Design Direction

## 1. Visual theme and atmosphere

Raindrop is a quiet reading workspace: warm paper surfaces, ink-blue controls, and crisp information density. Navigation stays utilitarian and sans-serif; article titles and reading content use a serif-led stack so the product feels like a reader rather than an admin template.

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

ASTRYX is the component source. Buttons, inputs, banners, navigation, dialogs, and loading feedback keep ASTRYX structure and accessibility. Raindrop changes theme tokens and domain layout only. The radius scale is `6px`, `10px`, `14px`, and pill; buttons use the `10px` tier and content surfaces use `14px`.

## 5. Layout principles

Setup and login use a calm two-region composition on desktop: product context beside a focused form. Ready state uses one `AppShell`; future reader views use one responsive `Layout`. Sections are mostly cardless, with surface elevation reserved for forms and reading content.

The reader keeps CommaFeed's efficiency grammar rather than its page shape: an unread-first source tree, a stable entry queue, fast keyboard traversal, snapshot-based bulk read actions, and non-disruptive new-entry notices. Raindrop modernizes this as a three-pane desktop workspace, a two-region medium layout, and single-task mobile routes with deep links and restored scroll anchors. AI summaries, translations, and plugin artifacts are secondary reader sidecars; original feed content remains the default and never waits on them.

## 6. Depth and elevation

Depth comes from background lightness steps and a restrained shadow on the active form surface. Borders are dividers, not decoration. Glass blur and decorative gradients are excluded.

## 7. Do and do not

- Do use ASTRYX props and tokens before business CSS.
- Do keep errors next to their fields and retain user input after failures.
- Do keep one accent color per screen.
- Do provide skip-to-content and visible focus behavior.
- Do not create local generic Button, Dialog, Selector, or input wrappers.
- Do not compress desktop side panels into mobile.
- Do not animate keyboard-driven navigation or routine list changes.
- Do not automatically merge newly arrived entries into the active queue or move the current selection.
- Do not make summary, translation, or plugin output a prerequisite for reading the original article.

## 8. Responsive behavior

- `>= 1100px`: full desktop workspace.
- `720px-1099px`: contextual two-region layout.
- `< 720px`: one task per route with `AppShell + MobileNav`.
- Mobile uses `100dvh`, safe-area padding, 44px minimum targets, and content-owned scrolling at 390x844 and 360x800.

## 9. Motion and implementation prompt

Pressable controls use a 100-160ms `scale(0.97)` response. Occasional drawers use an interruptible sub-300ms transform with `cubic-bezier(0.32, 0.72, 0, 1)`. Reduced motion removes spatial movement while preserving useful opacity and color feedback.

Implementation prompt: compose screens from ASTRYX 0.1.6, use canvas `oklch(0.975 0.008 85)`, ink `oklch(0.23 0.025 255)`, accent `oklch(0.42 0.09 250)`, button radius `10px`, surface radius `14px`, system sans controls, Charter plus explicit CJK serif reading text, and no custom generic controls.
