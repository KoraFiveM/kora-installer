# Panel web hardening (branch `fix/panel-web-hardening`)

Notes for whoever merges this: the CHANGELOG lines and the text meant for `docs/panel.md` (mostly section
12, "Security model"). Neither file is edited on this branch.

## CHANGELOG lines (Unreleased)

### Security
- Sign-in links now carry a one-time code (`/#code=...`, valid 10 minutes, works once) instead of the
  launch token, so the token no longer ends up in the browser history. Pasting the token still works.
- The session cookie is now named per port (`kora_session_<port>`) and is useless on its own: every
  request must also carry the session's secret, which the page keeps in its own storage. Browsers send
  cookies to every port of a host, so before this txAdmin, or a FiveM resource listening on the game port,
  could see the cookie and use it.
- Requests that came through a proxy or `tailscale serve` are refused (they would make every visitor look
  like the computer itself). Use `kora panel --tailscale` or an SSH tunnel instead.
- Failed remote sign-ins are counted per address (IPv6: per /64). Someone guessing tokens now only locks
  themselves out, not every remote user.
- Remote sessions can hold at most 8 live views (console, job progress, server status) at once, and all
  remote sessions together at most 64. Server status for every open page is now read once, however many
  pages are open. The fleet list is reused for 5 seconds.
- A remote session can only follow a job its role could have started (a viewer can no longer read the
  progress of an install, for example). "Already running" answers no longer show the running job's id.
- The fleet and other panels' servers (the hub) are now available on the computer that runs Kora only,
  as documented: they use the other panels' stored tokens.
- From another computer, the alert webhook address must be a public internet address (not this computer,
  a private network, link-local, CGNAT/Tailscale or "any" address). On the computer itself any address
  still works.

### Changed
- Opening the panel from another computer signs you in with your Tailscale account straight away when
  that account is allowed; otherwise the usual sign-in screen is shown.
- Settings > Remote access now suggests `kora panel --tailscale`.
- New API route `POST /api/v1/session/link` (local callers only) answers a one-time sign-in code.

## Text for docs/panel.md

### Signing in (replace the part that describes `/#token=`)

`kora panel` opens a link like `http://127.0.0.1:47120/#code=3f9c...`. The code in it works once and only
for 10 minutes; the page swaps it for a session and removes it from the address bar at once. If the link
has expired, run `kora panel` again (or `kora panel --print-link`) for a new one. You can still paste the
launch token into the sign-in box.

On another computer you sign in with your user token, or with nothing at all when the panel was started
with `kora panel --tailscale` and your Tailscale account is allowed: the page tries that by itself.

### API notes (section on sessions)

- `POST /api/v1/session/link` (local only: the launch token as `Authorization: Bearer`, or a local
  session) answers `{"code": "<32 hex>", "expires_in": 600}`. At most 32 codes are kept; making more drops
  the oldest.
- `POST /api/v1/session` takes `{"token": "..."}` (launch token locally, a user token remotely),
  `{"code": "..."}` (local only; uses up the code; wrong codes count towards the lockout like wrong tokens)
  or `{}` (remote, Tailscale identity). It answers `{"ok": true, "identity": {...}, "secret": "<64 hex>"}`
  and sets the cookie `kora_session_<port>` (HttpOnly, SameSite=Strict, Path=/, no expiry date).
- A request signed in by the cookie must also send the secret: header `X-Kora-Session: <secret>`, or for
  a GET (an `EventSource` cannot set headers) the query parameter `?ks=<secret>`. Without it the cookie is
  ignored (401). Requests with `Authorization: Bearer` (the CLI, a hub) are not affected.
- `POST /api/v1/session/logout` ends the session and clears the cookie; the page forgets the secret.

### 12. Security model (additions)

- **Cookies and other services on the same host.** Browsers send a cookie to every port of a host, so the
  panel's cookie also reaches txAdmin (port 40120) and the FiveM server's own web server (port 30120),
  where any resource can log request headers. The cookie alone is therefore not enough: each session has
  a second secret that the page keeps in the browser's storage for this exact address and port, and sends
  in the `X-Kora-Session` header. Someone who only sees the cookie cannot use it. The cookie name contains
  the port, so two panels on one computer do not overwrite each other.
- **Sign-in links.** The link printed by `kora panel` holds a one-time code, not the launch token. It
  works once, for 10 minutes, and only on the computer that runs Kora.
- **No proxies.** Requests carrying `X-Forwarded-For`, `X-Forwarded-Host`, `X-Forwarded-Proto`,
  `Forwarded`, `X-Real-IP`, `CF-Connecting-IP`, `True-Client-IP`, `Tailscale-User-Login` or
  `Tailscale-User-Name` are refused with 403. A reverse proxy or `tailscale serve` connects from
  127.0.0.1, so everyone would look like the owner. To reach the panel from your own devices, use
  `kora panel --tailscale`; for anything else, use an SSH tunnel.
- **Guessing tokens.** Failed remote sign-ins are counted per address (IPv6 per /64): 10 failures within
  a minute lock that address out for a minute. Failures on the computer itself have their own counter,
  so strangers can never lock the owner out.
- **Limits for remote sessions.** At most 8 live views per account and 64 in total (`429` with a plain
  message beyond that). The server-status stream is read once for all pages; the fleet list is reused for
  5 seconds.
- **Jobs.** A remote session can read a job (`GET /jobs/{id}`, its event stream, an install log) only if
  its role could have started that job; jobs of local-only operations are never shown remotely.
- **Hub.** The fleet, the node list and controlling a node's servers are local only.
- **Alert webhooks.** Set from another computer, the address must resolve to public internet addresses
  only. The check is made when the address is saved; on the computer itself any address is allowed.
