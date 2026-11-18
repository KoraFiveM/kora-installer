# Accessibility

The panel (`web/`) aims at **WCAG 2.2 level AA**. This page says what was checked and how, what is known to fall short, and
how to keep it that way. It is honest about the second part: a scanner finds a third of the problems, and nobody has tested
the panel with a real screen reader yet.

## How it is kept: the axe gate

Every browser test of the mock layer scans the page it reaches with [axe](https://github.com/dequelabs/axe-core) (`scan(label)` in
`web/e2e/fixtures.ts`). **The gate fails on `moderate`, `serious` and `critical` violations** (only `minor` best-practice findings are
tolerated; `E2E_AXE_REPORT=1` prints them). **No rule is switched off** (`AXE_EXCLUDED` in the fixture is empty: keep it that way,
and write the reason next to any rule you ever add). The mock layer's own switcher (`#kora-mock-switcher`) is outside the scan: it
is not part of the panel.

`web/e2e/mock/a11y.spec.ts` makes sure the scanner **reaches** everything: all twelve tabs of two servers, every page, and every
dialog (add an existing server, delete a server, check / restore / delete a backup, rename / replace / delete a licence key,
the support bundle, change licence key, restart, add a node, the branch dialog), each opened, scanned and closed with Escape
(focus must come back to the button that opened it). The sign-in and set-up screens, the recovery codes, the wizard's every step
and the failed-step screens are scanned by their own journeys. The scan also runs in German and in the pseudo-language
(`web/e2e/mock/i18n.spec.ts`), which found a real *label in name* problem (WCAG 2.5.3) in a button whose visible text was
not at the start of its `aria-label`.

What axe cannot see is checked by `a11y.spec.ts` and the keyboard journeys:

- exactly one `h1` and one `main` per page, a labelled `nav`, a page title ("Settings · Kora") per route, and a heading
  outline that never skips a level (every page and tab);
- **route changes**: focus moves to the new page's heading and the title follows (`useRouteAnnouncer`), the skip link is the
  first stop and moves focus into `main`;
- a dialog has a name and a description, focus is inside it, Tab stays in it, Escape closes it and focus returns;
- **320 px wide** (also what 400 % zoom gives): eight pages and a main journey have no sideways scrolling and pass axe; the
  dialog fits the screen;
- `prefers-reduced-motion`: transitions and animations are switched off (`index.css`);
- **keyboard-only journeys** with no mouse click and no `fill()`: the install wizard to "All done", a failed step ("Try this step
  again", "Skip this step"), a dialog, the server tabs with the arrow keys, **the resource page** (sign in to GitHub, the diff, save
  a change with a message, the pull-request tick and button), **backups** (make one, restore with its confirmation step, Escape back to
  the button), **Settings** (turn remote access on and invite a person), and **sign-in with a one-time code** (a refusal is read
  out as an alert, then success).

## What was changed in the pass

- **Headings:** card titles are `h2` (`CardTitle`, `as` for the rare other level); the sign-in and set-up cards are the page's `h1`.
- **Landmarks:** the alerts, the monitoring banner and the "replace your token link" notice moved inside `main`; the sign-in
  screens are a `main`; the navigation wraps instead of overflowing.
- **Skip link, route focus and titles** (`AppShell`, `useRouteAnnouncer`, `usePageTitle`).
- **Live regions** (`Announcer`): progress is announced at its start, at each quarter and at its end, and a step list says when a
  step starts, not on every percent. A bar that moves every second used to be inside an `aria-live` region and would have been
  read out every second. Failures stay in `role="alert"` (`ErrorNotice`, `Alert tone="danger"`); everything else is polite.
- **Diff and pull-request files:** "Added:" / "Removed:" are in the text (hidden from the eye), not only colour; `+`/`-` stay.
  The red used for removed lines was a Tailwind class that did not exist (`text-red`): it is `text-danger-text` now.
- **Contrast** (checked numerically, see below): the border of form fields and outline buttons (`--input`) was 1.6:1 and is now
  3.7:1; placeholder text was below 4.5:1 and now uses the muted token (5.9:1).
- **Reflow at 320 px:** the navigation wraps, a grid that could not shrink (backups), a button row, long addresses (`overflow-wrap`).
- **Tables:** the activity log has a caption, column headers with `scope` and a keyboard-reachable scroll region; the servers and
  fleet tables had captions and now `scope="col"`.
- **Reduced motion** respected everywhere (spinners, bars, the skeleton).
- **Target size (2.5.8):** the dismiss buttons are at least 24 px; axe's `target-size` rule passes on every page.
- **Language:** `<html lang>` follows the panel's language (`translations.md`); the language names in the picker carry their own `lang`.

### Colour contrast

The panel has one theme (dark). Tokens in `web/src/index.css`, ratios against the page `#0f0f0f` / card `#151515` / secondary `#202020`:

| text | on page | on card | on secondary |
| --- | --- | --- | --- |
| foreground `#ededed` | 16.4 | 15.6 | 13.9 |
| muted `#8e8e8e` | 5.9 | 5.6 | 5.0 |
| lime `#95ff00` | 15.1 | 14.4 | 12.9 |
| blue `#4aa8ff` (info) | 7.6 | 7.3 | 6.5 |
| green `#22c55e` (success) | 8.4 | 8.0 | 7.2 |
| amber `#f5a623` | 9.5 | 9.0 | 8.0 |
| orange `#e67e22` | 6.7 | 6.4 | 5.7 |
| danger text `#f87171` | 6.9 | 6.6 | 5.9 |
| steel `#95a5a6` | 7.5 | 7.1 | 6.4 |

All are at least 4.5:1 (AA). Solid red `#dc2626` is only used as a fill (3.8:1 as a text colour would fail, so text uses
`danger-text`); white text on it is 4.8:1. Primary button text `#0a1204` on lime is 15.1:1. The focus ring `#22c55e` is 8.4:1 on
the page. Field edges `#6e6e6e` are 3.7:1 (WCAG 1.4.11).

### Status is never colour alone

Badges carry words; the "not on its normal version" badge has an icon, words and a visually hidden "Not on its normal version:";
checklist lines have an icon and a word (OK / Needs fixing / Good to know); step lists say "Done", "In progress" or "Failed" in text
(hidden from the eye, read by screen readers); the diff says Added / Removed; a running server is "Running" in words.

## How to keep it (patterns, and where they live)

- **Forms:** `Field` (`components/Field.tsx`) ties label, hint and error together (`aria-describedby`, `aria-invalid`, an error in
  `role="alert"`); give inputs the right `autoComplete` (`username webauthn`, `current-password`, `new-password`, `one-time-code`);
  never use a placeholder as the label (placeholders here are examples).
- **Errors:** always `ErrorNotice` (an alert; one button per action with a handler; details folded away).
- **Progress and "it worked" messages:** `Announcer` with a message that changes only at milestones; never put `aria-live` on the
  thing that changes. A status text that appears after an action is `role="status"` (polite).
- **Dialogs:** `DialogContent` / `ConfirmDialog` (a title, a description, focus returned to the opener).
- **Titles:** one `h1` per page, `CardTitle` for the sections; the route announcer reads the `h1`.
- **Tables:** a caption (can be `sr-only`), `th scope`, and no layout tables.
- **Icons** next to words are `aria-hidden`; an icon-only button has an `aria-label` that **starts with the visible text** (2.5.3).
- **Text:** through `t()`/`fmt` so it can be translated and formatted for the language (`translations.md`).
- **Any new page or dialog:** add it to `a11y.spec.ts` (or reach it in a journey with `scan(...)`), and run `CI=1 npm run e2e`.

## Known gaps

- **No real screen reader or voice-control test.** Everything above is axe, the browser's accessibility tree and code reading.
  Live-region behaviour in particular (what NVDA, JAWS, VoiceOver and Orca say, and when) is unverified.
- **Wide data tables** (the servers list, the fleet) scroll sideways inside their card at 320 px. That is allowed for data tables
  (1.4.10 exempts them) but is not pleasant; a stacked layout would be better.
- **Focus after a control disappears** (for example the "Sign in to GitHub" button once it works) falls back to the page; moving it to
  the thing that replaced the button is not done everywhere.
- **Forced colours / Windows high contrast** was not tested; the panel relies on its own colours.
- **Light theme:** none; people who need a light page rely on the browser's own tools.
- **Server text** (progress lines, notes, resource descriptions) is English and sometimes long; it is not marked up beyond paragraphs.
- Tiny inline links inside sentences are below 24 px tall when they wrap (allowed by 2.5.8's inline exception).
- The console and log views are `<pre>` regions that scroll; they are focusable and labelled but dense for a screen reader.
