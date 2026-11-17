# Kora Panel web UI

The browser interface of `kora panel` (docs/panel-plan.md). Vite + React + TypeScript (strict) + Tailwind CSS v4 +
shadcn/ui-style components (copied into `src/components/ui/`) + React Router + TanStack Query. Dark theme only.
There are no external requests at run time: fonts are bundled, icons are lucide-react, and the Content-Security-Policy
the server sends (`default-src 'self'` ...) is respected (no inline scripts, no eval).

## Commands

```sh
cd web
npm ci --ignore-scripts      # or: npm install --ignore-scripts
npm run dev                  # dev server (http://localhost:5173) with the MOCK backend (all data is fake)
VITE_MOCK=0 npm run dev      # dev server against a real backend: /api is proxied to http://127.0.0.1:47120
npm run build                # tsc (strict) + vite build -> web/dist (hashed files under dist/assets/)
VITE_MOCK=1 npm run build    # a build that includes the mock (for demos); normal builds contain no mock data
npm run lint                 # eslint (flat config)
npm run size                 # after a build: first-load size (raw + gzip) and a check that the mock is not in the bundle
npm run e2e                  # browser tests (Playwright) against the mock build: see CONTRIBUTING.md, "Browser tests"
npm test                     # vitest + Testing Library (jsdom)
```

With a real backend, open `http://127.0.0.1:5173/#token=<token>` (dev server) or the URL `kora panel` prints.
With the mock any token works, except `bad`, which shows the "session expired" screen.

## How the mock works

`src/api/mock/` replaces `window.fetch` (only for `/api/v1/...`) and `window.EventSource` with in-app versions. It keeps
realistic state in memory: 3 servers (`kora-live` running production, `kora-dev` stopped development with 2 updates
available, `kora-old` with a missing folder), a catalogue with core / compatibility / bridges / creators groups,
snapshots, logs, an echoing console, and jobs that emit progress over time. Failure cases to try:

- Ports: `30120` is in use (Install wizard, Ports tab); starting a server whose port is taken answers `conflict`.
- Schedule tab, "The system account": `needs_elevation` with a command to copy.
- Install a server whose name starts with `fail`: it stops at the database step and offers "Try again"; the next visit to
  the wizard offers "Continue where you left off".
- Add resources: `ns_garage` fails to install. Ensure, webhook and GitHub (device code, approves after about 6 s) work.
- A folder containing `small` fails the disk check; the database password `wrong` fails "Test login".

The mock is switched on by a compile-time constant (`__KORA_MOCK__`, set in `vite.config.ts`): true for `npm run dev`
(unless `VITE_MOCK=0`) and for `VITE_MOCK=1` builds. `src/main.tsx` imports it dynamically behind that constant, so a
normal build drops it entirely (check: `grep -rl "kora-live" dist` finds nothing).

## Mock scenarios

Pick a scenario with `?scenario=<name>` (for example `http://localhost:5173/?scenario=fleet`). It is kept in
`sessionStorage`, so navigating keeps it; a new `?scenario=` replaces it. In mock builds a small amber "Mock: <name>"
button sits bottom-right: it opens a panel to change scenario, "Reset state" (reloads, dropping everything done in the
page) and a link to the styleguide. Waits and "random" values are seeded, so a scenario looks the same every time.

| Scenario | What it shows |
|---|---|
| `default` | The 3 servers above; everything works. |
| `empty` | No servers and no saved install answers: the first-run screens. |
| `fleet` | 31 servers: every mode x status x badge (30), 32-character names, ports above 60000, one added from an existing folder. For table density and overflow. |
| `trouble` | Everything fails realistically: start fails with a port conflict, the update preview finds dirty/diverged resources, apply fails after the first resource and rolls back, `needs_elevation` on schedule / Kora app / uninstall / self-update, the webhook test fails, console not enabled, no log files, GitHub signed out (and sign-in fails), disk check fails, database login wrong. |
| `slow` | Like `default`, but every request takes 1.5 to 3 s and jobs run 3x slower (loading states). |
| `offline` | The API answers 401: the "session expired" screen. |
| `down` | Every request fails with a network error: the "Kora is not answering" states. |

Code: `src/api/mock/scenarios.ts` (names, selection, behaviour), `data.ts` (seeds), `index.ts` (the backend),
`Switcher.tsx` / `boot.tsx` (the floating control). `?as=operator|remote-operator|remote-admin` (kept like the scenario; `?as=local` resets) shows the panel as another person sees it: a local operator, an operator signed in from another device, or a token administrator from another device. The mock follows the real backend's response shapes
(`crates/kora-web/src/handlers/*.rs`): `needs_elevation` commands are `["kora", ...]` without `sudo` (the UI shows
`sudo ` in front on Linux), uninstall plan ids are `<server>/service|firewall|registry|folder|database` and `app`.

## Styleguide

`/styleguide` (mock builds only, not linked in the sidebar; the floating Mock panel links to it) shows the brand colours
with hex and CSS variable, the semantic variables, the type scale in the three fonts, every primitive in
`src/components/ui/` (buttons in all variants with default / forced hover / forced focus / disabled / loading, inputs,
checkbox, radio, badges, alerts, card, dialog, tabs, progress), status and update chips, job progress in each state
(pending, running 40%, done, failed with kept files and log path, needs administrator rights), error notices, an empty
state, a table with long content, the console view and skeletons. The page is `src/pages/Styleguide.tsx`.

## Styling workflow

1. `npm run dev` (mock backend on) and open `http://localhost:5173/styleguide`.
2. Change colours and fonts in `src/index.css` (the `--kora-*` variables, the shadcn variables and `@theme inline`);
   change component looks in `src/components/ui/*.tsx` and `src/components/Status.tsx`. Hot reload updates the page.
3. Check real screens with `?scenario=fleet` (density, overflow), `?scenario=trouble` (error states), `?scenario=slow`
   (skeletons) and `?scenario=empty` (first run).
4. `npm run lint && npm test && npm run build` before committing. A normal build must stay free of mock code:
   `grep -rlE "kora-live|styleguide|scenario" dist` finds nothing (`VITE_MOCK=1 npm run build` includes it all).

## How auth works

1. `kora panel` opens `/#token=<token>`. `src/main.tsx` calls `bootstrapSession()` (`src/lib/session.ts`) **before** React
   and the router start, so the router never sees the hash (the app uses browser routing; the server has SPA fallback).
2. If the hash has a token it is removed from the address bar with `history.replaceState` (also when sign-in fails), and
   `POST /api/v1/session {token}` sets the HttpOnly cookie. The UI never reads or stores the token or the cookie.
3. Every request uses `credentials: 'same-origin'`; every non-GET request sends `X-Kora: 1` (`src/api/client.ts`).
4. Any `401` shows the "Session expired, or not signed in" screen (explains `kora panel`, has a field to paste a token).
5. API errors are `{"error": {"code", "message", "command"?}}`. `message` is shown as is; `needs_elevation` shows `command`
   as a copyable command line with an explanation.

## Layout

- `src/api/types.ts`: TypeScript mirror of the API (snake_case). `src/api/client.ts`: one function per route of table 2.1.
- `src/hooks/useJob.ts` follows a job (SSE, falls back to polling); `src/components/JobProgress.tsx` shows it.
- `src/pages/`: `ServersPage`, `ServerDetailPage` (+ `server/*` tabs), `InstallWizard` (+ `install/*`), `SettingsPage`.
- `src/components/ui/`: shadcn-style primitives.

## Brand tokens

`src/index.css`: the `--kora-*` variables (lime, charcoal, ink, mist, white, blue, success, amber, orange, danger, steel)
and the shadcn variables (`--background`, `--primary`, ...) mapped onto them, plus the Tailwind theme (`@theme inline`).
Lime is used only for the primary button and the active navigation item. Fonts: `src/fonts.css` (latin woff2 from the
`@fontsource` packages: Inter Tight for headings, Inter for body, JetBrains Mono for console and code).

## Assumptions about the API (the backend must match)

The shapes are in `src/api/types.ts` and the routes in `src/api/client.ts`. Where the plan is silent, the UI assumes:
enums are snake_case strings (`use_existing`, `full_restart`, `up_to_date`, step status `pending|running|done|skipped|failed`;
capitalised Rust-style step statuses and `{"Resource": {...}}` update events are also tolerated); list routes wrap arrays
(`{servers}`, `{files}`, `{resources}`, `{snapshots}`, `{owners}`); job `error` is `{code, message, command?, kept?, log_path?}`;
`github_login` answers `202 {job, code, url}`.

## Code splitting

The first load is the shell and the server list; everything else is fetched when first opened (`src/App.tsx`: the install
wizard, a server's page, Fleet, Settings and the System check are lazy routes; `src/pages/ServerDetailPage.tsx`: each server
tab except Overview is lazy). React, the router, TanStack Query, Radix and the icons are separate vendor chunks
(`vite.config.ts`), so a browser keeps them cached across Kora updates. While a file is loading the shell shows a loading
state (`RouteLoading`). The panel binary embeds the UI, so after Kora updates itself an old tab asks for chunk files that
no longer exist (the panel answers 404 for them): the error boundary then shows "This page needs to be reloaded" with a
Reload button (`src/lib/chunkError.ts`, `src/components/ErrorBoundary.tsx`) instead of a blank page. `npm run size` prints
the first-load size and fails if it grows past the budget in `scripts/bundle-size.mjs`.
