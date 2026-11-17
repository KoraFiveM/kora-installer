// Mock data and routes for the tools, the System check and "start the database". Hooked up from
// mock/index.ts with one `installToolchainMock(...)` call (kept apart so edits do not collide).
import type * as T from '../types'

type Body = Record<string, unknown>
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>) => void
type FailCtor = new (status: number, code: string, message: string, command?: string[], extra?: Partial<T.JobError>) => Error
type StartJob = (op: string, server: string | null, run: (emit: (e: unknown) => void) => Promise<unknown>) => T.JobStarted

interface Hooks {
  route: Route
  startJob: StartJob
  Fail: FailCtor
  sleep: (ms: number) => Promise<void>
  /** The GitHub state the rest of the mock shows (so the System check agrees with the GitHub tab). */
  github: () => T.GithubStatus
  /** The `trouble` scenario: things are broken in realistic ways. */
  trouble: () => boolean
  /** The `windows` scenario: the panel asks Windows for administrator permission itself. */
  windows?: () => boolean
}

const ghMissing = { code: 'github.gh_missing', message: 'The GitHub CLI (gh) is not installed. Kora needs it to work with GitHub.', actions: [{ kind: 'install_tool', tool: 'gh' }, { kind: 'open_link', url: 'https://cli.github.com' }] } satisfies T.Problem

interface State {
  git: boolean
  gh: boolean
  dbRunning: boolean
  attempts: number
}
const state: State = { git: true, gh: true, dbRunning: true, attempts: 0 }

/** Start a scenario from a clean world: in `trouble` the GitHub CLI is missing and the database is stopped. */
export function resetToolchainMock(trouble: boolean): void {
  Object.assign(state, { git: true, gh: !trouble, dbRunning: !trouble, attempts: 0 })
}

/** Is the GitHub CLI installed in the mock world (the GitHub status route reads this)? */
export const mockGhInstalled = (): boolean => state.gh

const tool = (t: 'git' | 'gh'): T.ToolStatus => ({
  tool: t,
  name: t,
  label: t === 'git' ? 'Git' : 'GitHub CLI',
  installed: state[t],
  version: state[t] ? (t === 'git' ? '2.45.1' : '2.104.0') : null,
  source: state[t] ? (t === 'gh' ? 'private' : 'system') : null,
  path: state[t] ? (t === 'gh' ? '/home/sam/.kora/tools/gh/bin/gh' : '/usr/bin/git') : null,
  broken: false,
  can_install: true,
})

export function installToolchainMock({ route, startJob, Fail, sleep, github, trouble, windows }: Hooks): void {
  route('GET', '/tools', () => ({ tools: [tool('git'), tool('gh')] }))

  route('POST', '/tools/{tool}/install', (m) => {
    const t = decodeURIComponent(m[1])
    if (t !== 'git' && t !== 'gh') throw new Fail(404, 'not_found', 'Kora can install `git` and `gh` only.')
    return startJob('tool_install', null, async (emit) => {
      emit({ type: 'progress', message: `Downloading ${tool(t).label}`, fraction: 0.1 })
      await sleep(250)
      state.attempts += 1
      // The trouble scenario: the first download fails (offline), the second works.
      if (trouble() && state.attempts === 1) {
        throw new Fail(502, 'tools.download_failed', `Kora could not download ${t === 'git' ? 'Git' : 'the GitHub CLI (gh)'}. Check your internet connection (or proxy) and try again.`, undefined, {
          details: 'could not download the file from any source (github.com: connection refused)',
          actions: [{ kind: 'retry' }, { kind: 'open_link', url: t === 'git' ? 'https://git-scm.com/downloads' : 'https://cli.github.com' }],
        })
      }
      emit({ type: 'progress', message: `Downloading ${tool(t).label} (60%)`, fraction: 0.5 })
      await sleep(250)
      emit({ type: 'progress', message: 'Unpacking', fraction: 0.86 })
      await sleep(150)
      state[t] = true
      emit({ type: 'progress', message: `${tool(t).label} is ready`, fraction: 1 })
      const s = tool(t)
      return { tool: t, version: s.version, path: s.path, source: s.source }
    })
  })

  route('GET', '/system/check', () => {
    const g = github()
    const gitFix: T.Problem = { code: 'install.tool_missing', message: 'Git is not installed. Kora needs it to download and update resources.', actions: [{ kind: 'install_tool', tool: 'git' }, { kind: 'open_link', url: 'https://git-scm.com/downloads' }] }
    const toolItem = (id: 'git' | 'gh', label: string, fix: T.Problem): T.CheckItem => {
      const t = tool(id)
      return t.installed
        ? { id, label, state: 'ok', summary: `Installed ${t.version} (${t.source === 'private' ? "Kora's own copy" : 'found on this computer'}).`, problem: null }
        : { id, label, state: 'problem', summary: 'Not installed.', problem: fix }
    }
    const github_: T.CheckItem =
      g.state === 'gh_missing' ? { id: 'github', label: 'GitHub sign-in', state: 'info', summary: 'Needs the GitHub CLI first.', problem: null }
      : g.state === 'signed_out' ? { id: 'github', label: 'GitHub sign-in', state: 'problem', summary: 'Not signed in.', problem: g.problem ?? null }
      : g.state === 'missing_scopes' ? { id: 'github', label: 'GitHub sign-in', state: 'problem', summary: 'Signed in, but without the permissions Kora needs.', problem: g.problem ?? null }
      : { id: 'github', label: 'GitHub sign-in', state: 'ok', summary: `Signed in${g.user ? ` as ${g.user}` : ''}.`, problem: null }
    const db: T.CheckItem = state.dbRunning
      ? { id: 'database', label: 'Database', state: 'ok', summary: 'mariadb 11.4.2 is running.', problem: null }
      : { id: 'database', label: 'Database', state: 'problem', summary: 'mariadb 11.4.2 is installed but not running.', problem: { code: 'install.db_stopped', message: 'The database server mariadb is installed but not running.', actions: [{ kind: 'start_database' }] } }
    const items: T.CheckItem[] = [
      toolItem('git', 'Git', gitFix),
      toolItem('gh', 'GitHub CLI', ghMissing),
      github_,
      db,
      { id: 'admin', label: 'Administrator rights', state: 'info', summary: windows?.() ? 'Kora runs with your normal rights. When something needs administrator rights, Windows asks you for administrator permission on this computer: once for a whole install, once for other actions.' : 'Kora runs with your normal rights. For the steps that need root it shows you the command to run.', problem: null },
      { id: 'service', label: 'Background service', state: 'info', summary: 'Not set up: the panel runs only while it is open.', problem: null },
    ]
    return { items, ok: items.every((i) => i.state !== 'problem') } satisfies T.SystemCheck
  })

  route('POST', '/system/database/start', () => {
    if (state.dbRunning) throw new Fail(409, 'conflict', 'The database server is already running.')
    if (trouble()) {
      throw new Fail(403, 'needs_elevation', 'Starting the database server mariadb needs administrator rights. Run this command as administrator:', ['kora', 'check', '--start-database'], {
        actions: [{ kind: 'request_admin' }, { kind: 'show_command', command: ['kora', 'check', '--start-database'], elevated: true }],
      })
    }
    return startJob('db_start', null, async (emit) => {
      if (windows?.()) emit({ type: 'status', message: 'Windows is asking for permission on the computer that runs Kora. Choose Yes to carry on.' })
      emit({ type: 'status', message: 'Starting the database server mariadb' })
      await sleep(400)
      state.dbRunning = true
      return { message: 'The database server mariadb is running.' }
    })
  })
}

/** For the wizard's database check: a failed login on a stopped database carries the fix. */
export function mockDbStoppedProblem(): T.Problem | null {
  return state.dbRunning ? null : { code: 'install.db_stopped', message: 'The database server mariadb is installed but not running.', actions: [{ kind: 'start_database' }] }
}

/** Tests: the database server is installed but stopped (and starting it needs no administrator rights outside `trouble`). */
export function stopMockDatabase(): void {
  state.dbRunning = false
}
