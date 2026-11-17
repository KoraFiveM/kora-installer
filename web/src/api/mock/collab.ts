// Mock routes for the GitHub loop of a resource: changes, log, commit, push, pull requests (details, checks, review,
// merge), issues, releases, making your own copy, and the "which versions" setting. Kept apart from the main mock.
//
// Demo world (see docs/panel.md): kora_jobs has local changes; PR #41 has failing checks, #38 is a draft with pending checks,
// #3 passes; a pull request on the original can only be merged by someone with write access (not this account); kora_phone
// cannot be pushed to (no rights); signed out gives the usual guided GitHub sign-in problem.
import type * as T from '../types'

type Body = Record<string, unknown>
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>) => void
type FailCtor = new (status: number, code: string, message: string, command?: string[], extra?: Partial<T.JobError>) => Error
type StartJob = (op: string, server: string | null, run: (emit: (e: unknown) => void) => Promise<unknown>) => T.JobStarted

export interface CollabDeps {
  find: (name: string) => T.ServerSummary
  installed: (server: string) => string[]
  signedIn: () => boolean
  /** The files of a resource changed (restart-needed notice). */
  onChanged: (server: string, id: string, why: string) => void
  /** Called after "which versions" or "make my own copy" changed a server's record. */
  onServerChanged: (s: T.ServerSummary) => void
}

const LOGIN = 'sam-rp'
const ORIGINAL = (id: string) => `KoraFiveM/${id}`
const when = (d: number) => new Date(Date.UTC(2026, 2, 10) - d * 86400_000).toISOString()

const mkPr = (number: number, over: Partial<T.PrView>): T.PrView => ({
  number,
  title: 'A change',
  body: 'What this does, in a few words.',
  author: 'jo-dev',
  state: 'OPEN',
  draft: false,
  url: `https://github.com/KoraFiveM/kora_core/pull/${number}`,
  repo: 'KoraFiveM/kora_core',
  base: 'dev',
  head: `branch-${number}`,
  created: when(5),
  updated: when(1),
  review: 'REVIEW_REQUIRED',
  checks: 'passing',
  check_list: [{ name: 'build', state: 'passing', url: 'https://github.com/KoraFiveM/kora_core/actions/runs/1' }],
  mergeable: 'MERGEABLE',
  files: [{ path: 'client/main.lua', additions: 12, deletions: 3 }],
  comments: [{ author: 'lee', body: 'Looks fine to me.', created: when(2), kind: 'comment' }],
  merge_methods: ['merge', 'squash'],
  can_merge: false,
  acting_as: LOGIN,
  ...over,
})

export const PR_VIEWS: Record<number, T.PrView> = {
  41: mkPr(41, { title: 'Fix the payroll rounding', author: 'sam-rp', head: 'fix-payroll', checks: 'failing', check_list: [{ name: 'build', state: 'passing' }, { name: 'tests', state: 'failing', url: 'https://github.com/KoraFiveM/kora_core/actions/runs/2' }], review: 'CHANGES_REQUESTED', comments: [{ author: 'lee', body: 'The rounding is still off for negative amounts.', created: when(1), kind: 'changes_requested' }] }),
  38: mkPr(38, { title: 'New inventory layout', draft: true, checks: 'pending', check_list: [{ name: 'build', state: 'pending' }], review: '', files: [{ path: 'html/inventory.html', additions: 140, deletions: 80 }, { path: 'client/inventory.lua', additions: 30, deletions: 12 }] }),
  3: mkPr(3, { title: 'My experiment', author: LOGIN, repo: 'sam-rp/kora_core', url: 'https://github.com/sam-rp/kora_core/pull/3', head: 'experiment', review: 'APPROVED', can_merge: true, merge_methods: ['squash'] }),
}

export function installCollabMock(route: Route, startJob: StartJob, Fail: FailCtor, d: CollabDeps) {
  const forked = new Set<string>() // `${server}/${resource}`
  const files = new Map<string, T.ChangedFile[]>() // changes by `${server}/${resource}`
  const commits = new Map<string, T.LogEntry[]>()
  const pushed = new Set<string>()
  const extra = new Map<string, T.PrComment[]>() // comments added in the demo, by `${server}/${number}`
  const merged = new Set<number>()
  const key = (s: string, r: string) => `${s}/${r}`
  const signedOut = () => new Fail(403, 'github.signed_out', 'You are not signed in to GitHub.', undefined, { actions: [{ kind: 'sign_in_github', scopes: [] }] })
  const known = (name: string, resource: string) => {
    const s = d.find(name)
    if (!d.installed(s.name).includes(resource)) throw new Fail(404, 'not_found', `'${resource}' is not installed on this server.`)
    return s
  }
  const dirty = (server: string, id: string): T.ChangedFile[] => {
    const k = key(server, id)
    if (!files.has(k)) files.set(k, id === 'kora_jobs' ? [{ path: 'config.lua', status: 'modified' }, { path: 'client/main.lua', status: 'modified' }, { path: 'notes.txt', status: 'new' }] : [])
    return files.get(k) ?? []
  }
  const log = (server: string, id: string): T.LogEntry[] => {
    const k = key(server, id)
    if (!commits.has(k)) commits.set(k, [{ sha: 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678', subject: 'Fix the contact list', author: 'jo-dev', when: 1_773_000_000 }, { sha: '0f9e8d7c6b5a49382716f5e4d3c2b1a098765432', subject: 'Add the phone app', author: 'jo-dev', when: 1_772_000_000 }])
    return commits.get(k) ?? []
  }
  const guarded = (s: T.ServerSummary) => { if (!d.signedIn()) throw signedOut(); return s }
  const needsFork = (id: string) => new Fail(422, 'collab.needs_fork', 'Pushing needs your own copy (a fork) of this resource on GitHub, and this one is a plain copy of the original. Choose "Make my own copy (fork)" first.', undefined, { actions: [{ kind: 'make_fork', resource: id }] })
  const isFork = (s: T.ServerSummary, id: string) => s.source === 'fork' || forked.has(key(s.name, id))

  route('GET', '/servers/{s}/resources/{name}/changes', (m) => {
    const s = known(decodeURIComponent(m[1]), decodeURIComponent(m[2]))
    const id = decodeURIComponent(m[2])
    const f = dirty(s.name, id)
    const diff = f.filter((x) => x.status !== 'new').map((x) => `diff --git a/${x.path} b/${x.path}\n--- a/${x.path}\n+++ b/${x.path}\n@@ -1,3 +1,4 @@\n Config = {}\n-Config.Pay = 100\n+Config.Pay = 120\n+Config.Bonus = true\n`).join('\n')
    return { branch: 'dev', files: f, count: f.length, diff, diff_truncated: false, ahead: pushed.has(key(s.name, id)) ? 0 : 1, behind: 0, repos: { original: ORIGINAL(id), fork: isFork(s, id) ? `${LOGIN}/${id}` : null } } satisfies T.LocalChanges
  })

  route('GET', '/servers/{s}/resources/{name}/log', (m) => {
    const s = known(decodeURIComponent(m[1]), decodeURIComponent(m[2]))
    return { branch: 'dev', commits: log(s.name, decodeURIComponent(m[2])) } satisfies T.Log
  })

  route('POST', '/servers/{s}/resources/{name}/commit', (m, b) => {
    const id = decodeURIComponent(m[2])
    const s = known(decodeURIComponent(m[1]), id)
    const message = String(b.message ?? '').trim()
    if (!message) throw new Fail(400, 'branch.bad_name', 'Write a message for the commit first.')
    const f = dirty(s.name, id)
    if (f.length === 0) throw new Fail(422, 'collab.nothing_to_commit', 'There is nothing to save: no file in this resource has changed.')
    files.set(key(s.name, id), [])
    const sha = Math.random().toString(16).slice(2).padEnd(40, '0').slice(0, 40)
    log(s.name, id).unshift({ sha, subject: message.split('\n')[0], author: 'You', when: Math.floor(Date.now() / 1000) })
    pushed.delete(key(s.name, id))
    return { commit: sha, files: f.length, branch: 'dev' } satisfies T.Committed
  })

  route('POST', '/servers/{s}/resources/{name}/push', (m) => {
    const id = decodeURIComponent(m[2])
    const s = known(decodeURIComponent(m[1]), id)
    if (!isFork(s, id)) throw needsFork(id)
    guarded(s)
    if (id === 'kora_phone') throw new Fail(422, 'collab.no_push_rights', `Your GitHub account is not allowed to push to ${LOGIN}/${id}. Ask the owner of that repository to give you write access, or push to your own copy.`, undefined, { actions: [{ kind: 'open_link', url: `https://github.com/${LOGIN}/${id}/settings/access` }] })
    pushed.add(key(s.name, id))
    return { branch: 'dev', to: `${LOGIN}/${id}`, new_branch: false, acting_as: LOGIN } satisfies T.Pushed
  })

  route('POST', '/servers/{s}/resources/{name}/new-branch', (m, b) => {
    const id = decodeURIComponent(m[2])
    const s = known(decodeURIComponent(m[1]), id)
    const name = String(b.name ?? '')
    if (!/^[A-Za-z0-9._/-]{1,200}$/.test(name) || name.startsWith('-') || name.startsWith('pr/') || name.includes('..')) throw new Fail(400, 'branch.bad_name', `'${name}' is not a branch name Kora can use.`)
    d.onChanged(s.name, id, `moved to branch ${name}`)
    return { kind: 'branch', branch: name, changed_files: dirty(s.name, id).map((x) => x.path), changed_count: dirty(s.name, id).length }
  })

  route('GET', '/servers/{s}/resources/{name}/prs/{number}', (m, _b, q) => {
    const s = known(decodeURIComponent(m[1]), decodeURIComponent(m[2]))
    guarded(s)
    const n = Number(m[3])
    const v = PR_VIEWS[n]
    if (!v) throw new Fail(422, 'branch.pr_not_found', `There is no pull request #${n} on ${ORIGINAL(decodeURIComponent(m[2]))}. Check the number.`)
    void q
    const own = extra.get(key(s.name, String(n))) ?? []
    const trying = s.name && n === 41 ? { branch: 'pr/41', behind: 2, ahead: 0 } : undefined
    return { ...v, state: merged.has(n) ? 'MERGED' : v.state, comments: [...v.comments, ...own], trying } satisfies T.PrView
  })

  const comment = (m: RegExpMatchArray, b: Body, kind: string): T.Acted => {
    const s = known(decodeURIComponent(m[1]), decodeURIComponent(m[2]))
    guarded(s)
    const text = String(b.body ?? '').trim()
    if (!text) throw new Fail(400, 'branch.bad_name', 'Write a comment first.')
    const k = key(s.name, m[3])
    extra.set(k, [...(extra.get(k) ?? []), { author: LOGIN, body: text, created: new Date().toISOString(), kind: 'comment' }])
    return { message: 'Your comment was added.', acting_as: LOGIN, url: `https://github.com/KoraFiveM/x/${kind}/${m[3]}#issuecomment-1` }
  }
  route('POST', '/servers/{s}/resources/{name}/prs/{number}/comment', (m, b) => comment(m, b, 'pull'))
  route('POST', '/servers/{s}/resources/{name}/issues/{number}/comment', (m, b) => comment(m, b, 'issues'))

  route('POST', '/servers/{s}/resources/{name}/prs/{number}/review', (m, b) => {
    const s = known(decodeURIComponent(m[1]), decodeURIComponent(m[2]))
    guarded(s)
    const action = String(b.action ?? '')
    if (!['approve', 'request_changes', 'comment'].includes(action)) throw new Fail(400, 'bad_request', 'A review is "approve", "request_changes" or "comment".')
    if (action !== 'approve' && !String(b.body ?? '').trim()) throw new Fail(400, 'branch.bad_name', 'Write the reason first.')
    if (action === 'approve' && PR_VIEWS[Number(m[3])]?.author === LOGIN) throw new Fail(422, 'collab.merge_refused', 'GitHub does not let an account approve its own pull request.')
    return { message: action === 'approve' ? 'You approved the pull request.' : action === 'request_changes' ? 'You asked for changes.' : 'Your review comment was added.', acting_as: LOGIN, url: null } satisfies T.Acted
  })

  route('POST', '/servers/{s}/resources/{name}/prs/{number}/merge', (m, b) => {
    const s = known(decodeURIComponent(m[1]), decodeURIComponent(m[2]))
    guarded(s)
    const n = Number(m[3])
    const v = PR_VIEWS[n]
    if (!v || merged.has(n)) throw new Fail(422, 'collab.merge_refused', `Pull request #${n} is merged, so it cannot be merged again.`)
    if (!v.can_merge) throw new Fail(422, 'collab.merge_refused', `Your GitHub account (${LOGIN}) is not allowed to merge into ${v.repo}. Ask someone with write access.`, undefined, { actions: [{ kind: 'open_link', url: v.url }] })
    const method = String(b.method ?? v.merge_methods[0])
    if (!v.merge_methods.includes(method)) throw new Fail(422, 'collab.merge_refused', `${v.repo} does not allow merging with "${method}". It allows: ${v.merge_methods.join(', ')}.`)
    merged.add(n)
    return { message: `Pull request #${n} was merged (${method}).`, acting_as: LOGIN, url: v.url } satisfies T.Acted
  })

  route('GET', '/servers/{s}/resources/{name}/issues', (m) => {
    const id = decodeURIComponent(m[2])
    const s = known(decodeURIComponent(m[1]), id)
    guarded(s)
    return {
      repo: ORIGINAL(id),
      issues: [{ number: 7, title: 'The phone app crashes on a new contact', author: 'jo-dev', url: `https://github.com/${ORIGINAL(id)}/issues/7`, comments: 3, created: when(9), labels: ['bug'] }, { number: 5, title: 'Add a dark mode', author: 'lee', url: `https://github.com/${ORIGINAL(id)}/issues/5`, comments: 0, created: when(20), labels: [] }],
      new_issue_url: `https://github.com/${ORIGINAL(id)}/issues/new`,
    } satisfies T.Issues
  })

  route('GET', '/servers/{s}/resources/{name}/releases', (m) => {
    const id = decodeURIComponent(m[2])
    const s = known(decodeURIComponent(m[1]), id)
    guarded(s)
    const pinned = s.versions === 'released'
    const rel = (tag: string, name: string, days: number, notes: string, flags: Partial<T.ReleaseItem> = {}): T.ReleaseItem => ({ tag, name, published: when(days), prerelease: false, url: `https://github.com/${ORIGINAL(id)}/releases/tag/${tag}`, notes, pinned: false, newer: false, ...flags })
    return {
      repo: ORIGINAL(id),
      pinned_commit: pinned ? 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678' : null,
      releases: [rel('v0.4.0', 'Four', 3, 'New contact app.\nFixes for the payroll rounding.', { newer: pinned }), rel('v0.3.1', 'Three point one', 20, 'Bug fixes.', { pinned }), rel('v0.3.0', 'Three', 40, 'Initial release of the phone.')],
    } satisfies T.Releases
  })

  route('POST', '/servers/{s}/fork', (m, b) => {
    const s = d.find(decodeURIComponent(m[1]))
    if (!d.signedIn()) throw signedOut()
    const only = Array.isArray(b.resources) ? (b.resources as string[]) : null
    return startJob('resources_fork', s.name, async (emit) => {
      const list = d.installed(s.name).filter((r) => r.startsWith('kora_') && (!only || only.includes(r)) && !isFork(s, r))
      emit({ type: 'forked', done: 0, total: list.length, id: '' })
      for (const [i, id] of list.entries()) {
        forked.add(key(s.name, id))
        emit({ type: 'forked', done: i + 1, total: list.length, id })
      }
      const remaining = d.installed(s.name).filter((r) => r.startsWith('kora_') && !isFork(s, r))
      if (remaining.length === 0) {
        s.source = 'fork'
        d.onServerChanged(s)
      }
      return { owner: typeof b.owner === 'string' && b.owner ? b.owner : LOGIN, forked: list.map((id) => ({ resource: id, fork: `${LOGIN}/${id}`, message: `${id} now has your own copy (${LOGIN}/${id}).` })), skipped: [], failed: [], server_is_fork: s.source === 'fork' } satisfies T.ForkOutcome
    })
  })

  route('PUT', '/servers/{s}/versions', (m, b) => {
    const s = d.find(decodeURIComponent(m[1]))
    const v = b.versions
    if (v !== 'released' && v !== 'latest') throw new Fail(400, 'bad_request', 'The versions must be "released" or "latest".')
    s.versions = v
    s.mode = v === 'released' ? 'production' : 'development'
    d.onServerChanged(s)
    return { source: s.source, versions: v, message: v === 'released' ? 'This server now follows released versions. Check for updates to see which resources move to a release; nothing has changed yet.' : 'This server now follows the latest changes. Check for updates to see what is new; nothing has changed yet.' } satisfies T.VersionsSet
  })

  return {
    /** Does the resource have your own copy (for the page's `repos`)? */
    repos: (id: string, s: T.ServerSummary): T.Repos => ({ original: ORIGINAL(id), fork: isFork(s, id) ? `${LOGIN}/${id}` : null }),
    reset: () => {
      forked.clear()
      files.clear()
      commits.clear()
      pushed.clear()
      extra.clear()
      merged.clear()
    },
  }
}
