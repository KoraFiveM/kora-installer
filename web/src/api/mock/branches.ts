// Mock data and routes for switching branches and trying pull requests (kept apart from the main mock so edits do not collide).
// `installBranchesMock` is called once from mock/index.ts with the mock's own helpers.
import type * as T from '../types'

type Body = Record<string, unknown>
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>) => void
type FailCtor = new (status: number, code: string, message: string, command?: string[], extra?: Partial<T.JobError>) => Error
type StartJob = (op: string, server: string | null, run: (emit: (e: unknown) => void) => Promise<unknown>) => T.JobStarted

export interface BranchesDeps {
  find: (name: string) => T.ServerSummary
  installed: (server: string) => string[]
  signedIn: () => boolean
  /** Files of a resource changed (the page's "restart needed" notice). */
  onChanged?: (server: string, id: string, why: string) => void
}

interface Res {
  branch: string
  pr?: number
  title?: string
  dirty: string[]
  stashes: T.BranchStash[]
}

const DEFAULT = 'dev'
const FORK = ['dev', 'feature/new-ui', 'fix-contacts']
const UPSTREAM = ['dev', 'feature/new-ui', 'release']
const PRS: T.PullRequest[] = [
  { number: 41, title: 'Fix the payroll rounding', author: 'sam-rp', source: 'upstream', repo: 'KoraFiveM/kora_core', branch: 'fix-payroll', from: 'sam-rp/kora_core', draft: false, url: 'https://github.com/KoraFiveM/kora_core/pull/41', updated: '2026-03-02T10:00:00Z' },
  { number: 38, title: 'New inventory layout', author: 'jo-dev', source: 'upstream', repo: 'KoraFiveM/kora_core', branch: 'new-layout', from: 'jo-dev/kora_core', draft: true, url: 'https://github.com/KoraFiveM/kora_core/pull/38', updated: '2026-02-20T10:00:00Z' },
  { number: 3, title: 'My experiment', author: 'sam-rp', source: 'fork', repo: 'sam-rp/kora_core', branch: 'experiment', from: 'sam-rp/kora_core', draft: false, url: 'https://github.com/sam-rp/kora_core/pull/3', updated: '2026-03-01T10:00:00Z' },
]
const MERGED = 12
const RESTART = 'The files changed, but a running server still has the old version loaded. Restart the resource (or the server) to load the new one.'

const validBranch = (b: string) => /^[A-Za-z0-9._/-]{1,200}$/.test(b) && !b.startsWith('-') && !b.includes('..') && !b.endsWith('/') && !b.endsWith('.lock')

/** Adds the routes; the returned function forgets every switch (a scenario starts from a clean world). */
export function installBranchesMock(route: Route, startJob: StartJob, Fail: FailCtor, d: BranchesDeps): (() => void) & { row: (server: string, id: string) => T.ResourceBranch } {
  const state = new Map<string, Res>()
  const key = (s: string, r: string) => `${s}/${r}`
  const trackable = (s: string) => d.installed(s).filter((r) => r.startsWith('kora_'))
  const get = (server: string, r: string): Res => {
    const k = key(server, r)
    let v = state.get(k)
    if (!v) {
      v = { branch: DEFAULT, dirty: [], stashes: [] }
      if (r === 'kora_phone') v = { ...v, branch: 'fix-contacts' }
      if (r === 'kora_jobs') v = { ...v, dirty: ['config.lua', 'client/main.lua'] }
      state.set(k, v)
    }
    return v
  }
  const dev = (name: string): T.ServerSummary => {
    // Every server can switch a resource to a branch or a pull request now.
    return d.find(name)
  }
  const checkName = (s: T.ServerSummary, r: string) => {
    if (!trackable(s.name).includes(r)) throw new Fail(422, 'branch.unsupported', `${r} is not a resource Kora can switch (it is not a fork of yours).`)
  }
  const labelOf = (v: Res) => (v.branch === DEFAULT ? `the normal version (${DEFAULT})` : v.pr !== undefined ? `pull request #${v.pr}` : `branch ${v.branch}`)
  const row = (server: string, id: string): T.ResourceBranch => {
    const v = get(server, id)
    const state: T.BranchState = v.branch === DEFAULT ? 'default' : v.pr !== undefined ? 'pr' : 'branch'
    return { id, folder: id, state, branch: v.branch, pr: v.pr, default_branch: DEFAULT, label: labelOf(v), changed_count: v.dirty.length, stashes: v.stashes.length }
  }
  const where = (v: Res): T.SwitchResult['now'] => ({ kind: v.branch === DEFAULT ? 'default' : v.pr !== undefined ? 'pr' : 'branch', branch: v.branch, pr: v.pr, changed_files: v.dirty, changed_count: v.dirty.length })
  const signedOut = () => new Fail(403, 'github.signed_out', 'You are not signed in to GitHub.', undefined, { actions: [{ kind: 'sign_in_github' }] })

  /** Deal with changes of the person's own; `'ask'` when there is no choice yet. */
  const leave = (v: Res, going: string, changes: string): { stashed?: T.BranchStash } | 'ask' => {
    if (v.dirty.length === 0) return {}
    if (changes === 'stash') {
      const stash: T.BranchStash = { index: 0, message: `kept aside on ${v.branch} before switching to ${going}`, branch: v.branch }
      v.stashes = [stash, ...v.stashes].map((x, i) => ({ ...x, index: i }))
      v.dirty = []
      return { stashed: stash }
    }
    if (changes === 'discard') {
      v.dirty = []
      return {}
    }
    return 'ask'
  }
  const result = (id: string, v: Res, status: T.SwitchResult['status'], message: string, extra: Partial<T.SwitchResult> = {}): T.SwitchResult => ({
    resource: id,
    status,
    now: where(v),
    message,
    needs_restart: status === 'switched',
    restart_note: status === 'switched' ? RESTART : undefined,
    ...extra,
  })
  const move = (server: string, id: string, going: string, apply: (v: Res) => void, changes: string, title?: string): T.SwitchResult => {
    const v = get(server, id)
    const left = leave(v, going, changes)
    if (left === 'ask') return result(id, v, 'needs_choice', `${v.dirty.length} file${v.dirty.length === 1 ? ' has' : 's have'} changes of your own that haven't been saved anywhere. Choose what to do with them first.`)
    apply(v)
    d.onChanged?.(server, id, `moved to ${labelOf(v)}`)
    return result(id, v, 'switched', `Now on ${labelOf(v)}.${left.stashed ? ' Your changes were kept aside: you can bring them back later.' : ''}`, { stashed: left.stashed, title })
  }

  route('GET', '/servers/{s}/branches', (m) => {
    const s = dev(decodeURIComponent(m[1]))
    return { resources: trackable(s.name).map((r) => row(s.name, r)) }
  })

  route('GET', '/servers/{s}/resources/{name}/branches', (m) => {
    const s = dev(decodeURIComponent(m[1]))
    const id = decodeURIComponent(m[2])
    checkName(s, id)
    const v = get(s.name, id)
    return { resource: row(s.name, id), branches: { current: v.branch, default_branch: DEFAULT, local: [...new Set([DEFAULT, v.branch])], fork: FORK, upstream: UPSTREAM }, stashes: v.stashes, notes: [] } satisfies T.BranchChoices
  })

  route('GET', '/servers/{s}/resources/{name}/prs', (m) => {
    const s = dev(decodeURIComponent(m[1]))
    checkName(s, decodeURIComponent(m[2]))
    if (!d.signedIn()) throw signedOut()
    return { prs: PRS, notes: [] } satisfies T.PrChoices
  })

  route('POST', '/servers/{s}/resources/{name}/branch', (m, b) => {
    const s = dev(decodeURIComponent(m[1]))
    const id = decodeURIComponent(m[2])
    checkName(s, id)
    const changes = String(b.changes ?? 'ask')
    if (!['ask', 'stash', 'discard'].includes(changes)) throw new Fail(400, 'bad_request', 'What to do with changes must be "ask", "stash" or "discard".')
    const v = get(s.name, id)
    switch (b.action) {
      case 'default':
        if (v.branch === DEFAULT) return result(id, v, 'already', `Already on the normal version (${DEFAULT}).`)
        return move(s.name, id, DEFAULT, (x) => { x.branch = DEFAULT; x.pr = undefined; x.title = undefined }, changes)
      case 'branch': {
        const name = String(b.branch ?? '')
        if (!validBranch(name)) throw new Fail(400, 'bad_request', 'That is not a branch name Kora can use.')
        if (!FORK.includes(name) && !UPSTREAM.includes(name)) throw new Fail(422, 'branch.branch_gone', `There is no branch called '${name}' here any more. It may have been deleted or renamed on GitHub. Refresh the list to see what exists.`, undefined, { actions: [{ kind: 'retry' }] })
        if (v.branch === name) return result(id, v, 'already', `Already on branch ${name}.`)
        return move(s.name, id, name, (x) => { x.branch = name; x.pr = undefined; x.title = undefined }, changes)
      }
      case 'pr': {
        const text = String(b.pr ?? '').trim()
        const url = /^https:\/\/github\.com\/([\w.-]+\/[\w.-]+)\/pull\/(\d+)/.exec(text)
        const n = Number(url ? url[2] : text.replace(/^#/, ''))
        if (!Number.isInteger(n) || n <= 0 || (!url && !/^#?\d+$/.test(text))) throw new Fail(400, 'branch.bad_name', "That doesn't look like a pull request. Type its number (like 12) or paste its address from GitHub.")
        if (url && !['KoraFiveM/kora_core', 'sam-rp/kora_core'].includes(url[1])) throw new Fail(422, 'branch.pr_other_repository', `That pull request is on ${url[1]}, which is not this resource's repository or your fork of it. Kora can only try pull requests of those two.`, undefined, { actions: [{ kind: 'open_link', url: text }] })
        if (!d.signedIn()) throw signedOut()
        if (n === MERGED) throw new Fail(422, 'branch.pr_closed', `Pull request #${n} has already been merged, so there is nothing separate to try. Go back to the normal version to get it.`, undefined, { actions: [{ kind: 'open_link', url: `https://github.com/KoraFiveM/kora_core/pull/${n}` }] })
        const pr = PRS.find((p) => p.number === n)
        if (!pr) throw new Fail(422, 'branch.pr_not_found', `There is no pull request #${n} on KoraFiveM/kora_core or sam-rp/kora_core. Check the number.`)
        if (v.pr === n) return result(id, v, 'already', `Already trying pull request #${n}, and there is nothing new in it.`, { title: pr.title })
        return move(s.name, id, `pull request #${n}`, (x) => { x.branch = `pr/${n}`; x.pr = n; x.title = pr.title }, changes, pr.title)
      }
      default:
        throw new Fail(400, 'bad_request', 'Say what to do: "branch", "pr" or "default".')
    }
  })

  route('POST', '/servers/{s}/resources/{name}/stash/restore', (m, b) => {
    const s = dev(decodeURIComponent(m[1]))
    const id = decodeURIComponent(m[2])
    checkName(s, id)
    const v = get(s.name, id)
    if (v.dirty.length > 0) throw new Fail(422, 'branch.stash_needs_clean', 'This resource has changes of its own right now. Put them aside or discard them first, then bring the old ones back.', undefined, { actions: [{ kind: 'retry' }] })
    const st = v.stashes.find((x) => x.index === Number(b.index))
    if (!st) throw new Fail(422, 'branch.stash_gone', 'Those kept-aside changes are not there any more.')
    v.dirty = ['config.lua']
    d.onChanged?.(s.name, id, 'kept-aside changes brought back')
    return result(id, v, 'switched', 'Your changes are back (1 file). They also stay in the kept-aside list, so nothing can be lost.', { stashed: st })
  })

  route('POST', '/servers/{s}/branches/preview', (m, b) => {
    const s = dev(decodeURIComponent(m[1]))
    const branch = String(b.branch ?? '')
    if (!validBranch(branch)) throw new Fail(400, 'bad_request', 'That is not a branch name Kora can use.')
    return startJob('branch_bulk_preview', s.name, async (emit) => {
      const list = trackable(s.name)
      emit({ type: 'checked', done: 0, total: list.length, id: '' })
      const rows: T.BulkRow[] = list.map((id) => {
        const v = get(s.name, id)
        // The demo: kora_inventory is the one resource that does not have it.
        const has = (FORK.includes(branch) || UPSTREAM.includes(branch)) && !(id === 'kora_inventory' && branch !== DEFAULT)
        return { id, has, from: v.branch === branch ? undefined : FORK.includes(branch) ? 'origin' : 'upstream', already: v.branch === branch, now: row(s.name, id).label, changed_count: v.dirty.length }
      })
      rows.forEach((r, i) => emit({ type: 'checked', done: i + 1, total: rows.length, id: r.id }))
      return { branch, rows, have: rows.filter((r) => r.has).length, missing: rows.filter((r) => !r.has).length, notes: [] } satisfies T.BulkPreview
    })
  })

  route('POST', '/servers/{s}/branches/apply', (m, b) => {
    const s = dev(decodeURIComponent(m[1]))
    if (b.action !== 'default' && b.action !== 'branch') throw new Fail(400, 'bad_request', 'Say what to do: "branch" or "default".')
    const changes = String(b.changes ?? 'ask')
    const target = b.action === 'default' ? null : String(b.branch ?? '')
    if (target !== null && !validBranch(target)) throw new Fail(400, 'bad_request', 'That is not a branch name Kora can use.')
    const only = Array.isArray(b.resources) ? (b.resources as string[]) : null
    return startJob('branch_bulk_apply', s.name, async (emit) => {
      const list = trackable(s.name).filter((r) => !only || only.includes(r))
      const items: T.BulkItem[] = []
      emit({ type: 'switched', done: 0, total: list.length, id: '' })
      for (const [i, id] of list.entries()) {
        const v = get(s.name, id)
        const to = target ?? DEFAULT
        if (target !== null && id === 'kora_inventory') items.push({ id, status: 'skipped', message: `It has no branch called ${target}; left as it is.` })
        else if (v.branch === to) items.push({ id, status: 'already', message: `Already on ${labelOf(v)}.` })
        else {
          const r = move(s.name, id, to, (x) => { x.branch = to; x.pr = undefined; x.title = undefined }, changes)
          items.push({ id, status: r.status === 'needs_choice' ? 'needs_choice' : 'switched', message: r.message, stashed: r.stashed, files: r.status === 'needs_choice' ? r.now.changed_files : undefined })
        }
        emit({ type: 'switched', done: i + 1, total: list.length, id })
      }
      const count = (st: T.BulkStatus) => items.filter((x) => x.status === st).length
      const switched = count('switched')
      return { items, switched, needs_choice: count('needs_choice'), failed: 0, skipped: count('skipped'), needs_restart: switched > 0, restart_note: switched > 0 ? RESTART : undefined } satisfies T.BulkResult
    })
  })
  return Object.assign(() => state.clear(), { row })
}
