// Mock of the support bundle (kept apart from the main mock). `installSupportMock` is called once from mock/index.ts.
import type * as T from '../types'

type Body = Record<string, unknown>
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>) => void
type FailCtor = new (status: number, code: string, message: string) => Error

const NEVER = 'Kora never uploads this file anywhere. It is saved on this computer, and it leaves only if you send it yourself.'

const TEXTS: Record<string, string> = {
  'about.txt': 'Kora 0.8.0 (linux x86_64)\nOperating system: Ubuntu 24.04 (Linux 6.8)\nKora folder: ~/.kora\nFree space in it: 52.1 GB\n',
  'system-check.txt': 'Git: Ok - Installed (2.43.0)\nGitHub CLI: Ok - Installed\nGitHub sign-in: Problem - Not signed in.\nDatabase: Ok - mariadb 11.4 is running.\n',
  'tools.txt': 'git: git version 2.43.0\ngh: gh version 2.45.0\nmariadb: mariadb  Ver 15.1 Distrib 11.4\nmariadb-dump: mariadb-dump  Ver 10.19\n',
  'servers.json': '[\n  {"name": "kora-live", "dir": "/srv/fivem/live", "mode": "production", "ports": {"game": 30120, "txadmin": 40120}}\n]',
  'servers/kora-live/server.cfg': 'endpoint_add_tcp "0.0.0.0:30120"\nsv_licenseKey "<hidden: licence key>"\nset mysql_connection_string "<hidden: address with a login>"\nensure kora_core\n',
  'jobs.txt': '2026-03-17 04:15 UTC  server_control         kora-live        done\n2026-03-17 03:10 UTC  update_apply           kora-live        failed     git fetch failed for https://<hidden: token>@github.com/KoraFiveM/kora_hud.git\n',
  'audit.txt': '2026-03-17 04:15 UTC  server_control  kora-live  ok  local\n',
  'redactions.txt': 'about.txt: 0 values hidden\nservers/kora-live/server.cfg: 2 values hidden (address with a login, licence key)\njobs.txt: 1 value hidden (token)\n',
  'README.txt': 'This is a Kora support bundle.\n\nKora never uploads it: it only leaves the computer if the owner sends it.\n',
  'omitted.txt': 'servers/kora-live/secrets.cfg: left out, it holds the licence key and the database connection string\nservers/kora-live/database.json: left out, it holds the database login\nservers/kora-live/txData: left out, it holds txAdmin\'s admin accounts, password hashes and tokens\n',
}

export function installSupportMock(route: Route, Fail: FailCtor): void {
  let saved = 0
  route('POST', '/support/bundle/preview', () => {
    const files: T.BundleFile[] = Object.entries(TEXTS).map(([name, text]) => {
      const hidden = [...text.matchAll(/<hidden(?:: ([a-z ]+))?>/g)]
      return { name, bytes: text.length, redactions: name === 'redactions.txt' ? 0 : hidden.length, hidden: [...new Set(hidden.map((h) => h[1] ?? 'value'))] }
    })
    return {
      id: 'bundle-1',
      files,
      omitted: [
        { name: 'servers/kora-live/secrets.cfg', why: 'holds the licence key and the database connection string' },
        { name: 'servers/kora-live/database.json', why: 'holds the database login' },
        { name: 'servers/kora-live/txData', why: "holds txAdmin's admin accounts, password hashes and tokens" },
      ],
      total_bytes: files.reduce((n, f) => n + f.bytes, 0),
      folder: '/home/demo/.kora/support',
      never_uploaded: NEVER,
    } satisfies T.BundleListing
  })
  route('POST', '/support/bundle/file', (_m, b) => {
    const text = TEXTS[String(b.name)]
    if (b.id !== 'bundle-1') throw new Fail(422, 'failed', 'That support bundle is no longer in memory (they are kept for 15 minutes). Prepare it again.')
    if (text === undefined) throw new Fail(422, 'failed', 'That file is not in the bundle.')
    return { name: String(b.name), text } satisfies T.BundleFileText
  })
  route('POST', '/support/bundle', (_m, b) => {
    if (b.id !== 'bundle-1') throw new Fail(422, 'failed', 'That support bundle is no longer in memory (they are kept for 15 minutes). Prepare it again.')
    saved += 1
    return { file_name: `kora-support-20260317-0415${saved}.zip`, path: `/home/demo/.kora/support/kora-support-20260317-0415${saved}.zip`, bytes: 9_400, folder: '/home/demo/.kora/support' } satisfies T.BundleSaved
  })
  route('GET', '/support/bundle/{file}', () => {
    throw new Fail(404, 'not_found', 'The demo has no support bundle files to download.')
  })
  route('POST', '/support/summary', () => ({ text: `--- about.txt\n${TEXTS['about.txt']}--- tools.txt\n${TEXTS['tools.txt']}` }) satisfies T.SupportSummary)
}
