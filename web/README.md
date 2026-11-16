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
