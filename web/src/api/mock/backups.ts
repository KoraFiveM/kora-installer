// Mock of the backups of a server (kept apart from the main mock). `installBackupsMock` is called once from mock/index.ts.
// Scenario `trouble`: the database is not running, so a backup fails with a guided problem; `fleet` has many backups.
import type * as T from '../types'

type Body = Record<string, unknown>
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>) => void
type FailCtor = new (status: number, code: string, message: string, command?: string[], extra?: Partial<T.JobError>) => Error
type StartJob = (op: string, server: string | null, run: (emit: (e: unknown) => void) => Promise<unknown>) => T.JobStarted

const DAY = 86400
const nowS = () => Math.floor(Date.now() / 1000)
const stamp = (unix: number) => new Date(unix * 1000).toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15)
const PASSWORD = 'correct horse battery'

export interface BackupsMock {
  reset: (scenario: string) => void
}

interface Per {
  backups: T.BackupInfo[]
  settings: T.BackupSettings
  undo: string | null
  lastFailed: { at: number; why: string } | null
}

export function installBackupsMock(route: Route, startJob: StartJob, Fail: FailCtor, servers: () => T.ServerSummary[], sleep: (ms: number) => Promise<void>, serviceReady: () => boolean = () => false): BackupsMock {
  let per: Record<string, Per> = {}
  let dbStopped = false
  let dbLoginOk = true
  let adminGiven = false
  let n = 1

  const defaults = (): T.BackupSettings => ({ folder: '', retention: { keep_last: 7, keep_daily_days: 0 }, schedule: { enabled: false, cadence: 'daily', time: '03:30' }, resources: 'auto', stop_for_scheduled: false })
  const info = (server: string, at: number, o: Partial<T.BackupInfo> = {}): T.BackupInfo => {
    const id = stamp(at)
    const encrypted = o.encrypted ?? false
    return { id, file: `${server}-${id}.tar.gz${encrypted ? '.age' : ''}`, bytes: 48_000_000, sha256: 'ab'.repeat(32), encrypted, created_unix: at, label: 'manual', kora_version: '0.8.0', server, database: 'dumped', by_reference: 38, in_full: 2, stopped: false, too_new: false, ...o }
  }
  const get = (name: string): Per => {
    if (!servers().some((s) => s.name === name)) throw new Fail(404, 'not_found', `There is no server called "${name}".`)
    return (per[name] ??= { backups: [], settings: defaults(), undo: null, lastFailed: null })
  }
  const summary = (name: string): T.BackupSummary => {
    const p = get(name)
    const newest = p.backups.reduce((m, b) => Math.max(m, b.created_unix), 0)
    const age = nowS() - newest
    const ago = age < 3600 ? 'just now' : age < DAY ? `${Math.floor(age / 3600)} hours ago` : age < 2 * DAY ? 'yesterday' : `${Math.floor(age / DAY)} days ago`
    let warning: string | null = null
    if (p.lastFailed && p.lastFailed.at > newest) warning = `The last backup failed: ${p.lastFailed.why}.`
    else if (p.settings.schedule.enabled && newest && age > DAY + 6 * 3600) warning = 'The last backup is older than a day, but backups are meant to run every day.'
    return { last_ok_unix: newest || null, text: newest ? `Last backup ${ago}` : 'No backup yet', warning, scheduled: p.settings.schedule.enabled }
  }
  const folderOf = (name: string, s: T.BackupSettings) => `${s.folder || '/home/demo/.kora/server-backups'}/${name}`
  // With the administrator service the schedule is a job of the operating system, and the panel's own scheduler stands down.
  const byOs = (name: string) => serviceReady() && get(name).settings.schedule.enabled
  const view = (name: string): T.BackupsView => {
    const p = get(name)
    const u = p.undo ? p.backups.find((b) => b.id === p.undo) : undefined
    return {
      backups: [...p.backups].sort((a, b) => b.created_unix - a.created_unix),
      settings: p.settings,
      summary: summary(name),
      folder: folderOf(name, p.settings),
      default_folder: !p.settings.folder,
      free_bytes: 120_000_000_000,
      undo: u ? { id: u.id, created_unix: u.created_unix, encrypted: u.encrypted } : null,
      database: { kind: 'dumped', note: "The server's own database is copied as one consistent snapshot.", tool_missing: false },
      schedule_note: byOs(name)
        ? `Scheduled backups run as a job of your operating system (kora-backup-${name}), so they happen even when Kora's panel is closed, and one missed while the computer was off is made up when it is next on.`
        : "Scheduled backups run while Kora's background service is running (start it with `kora panel --background`). A backup that was missed is made up when it starts again. With Kora's administrator service installed they run as a job of your operating system instead, whether or not the panel is open.",
      schedule_by_os: byOs(name),
    }
  }
  const dbProblem = () => new Fail(422, 'backup.db_stopped', 'Kora cannot back up the database: the database server is not running.', undefined, { actions: [{ kind: 'start_database' }, { kind: 'retry' }] })
  const passFor = (b: T.BackupInfo, given: unknown) => {
    if (!b.encrypted) return
    if (!given) throw new Fail(422, 'backup.needs_passphrase', 'This backup is password protected. Enter the password to open it.')
    if (given !== PASSWORD) throw new Fail(422, 'backup.wrong_passphrase', 'That password does not open this backup. Check it and try again. (Kora does not keep passwords, so a lost one cannot be recovered.)')
  }
  const findB = (name: string, id: string) => {
    const b = get(name).backups.find((x) => x.id === id)
    if (!b) throw new Fail(404, 'backup.not_found', 'That backup is not there any more.')
    return b
  }

  route('GET', '/backups', () => ({ servers: servers().map((s) => ({ server: s.name, summary: summary(s.name) })) }) satisfies T.BackupsOverview)
  route('GET', '/servers/{s}/backups', (m) => view(decodeURIComponent(m[1])))
  route('POST', '/servers/{s}/backups', (m, b) => {
    const name = decodeURIComponent(m[1])
    const p = get(name)
    const pass = typeof b.passphrase === 'string' && b.passphrase ? b.passphrase : null
    if (b.for_move && !pass) throw new Fail(422, 'backup.weak_passphrase', 'A backup made to move a server needs a password: the file will travel, and it holds your licence key and database login.')
    if (pass && pass.length < 8) throw new Fail(422, 'backup.weak_passphrase', 'Use at least 8 characters for the password. A longer sentence is easier to remember and much harder to guess.')
    return startJob('backup_create', name, async (emit) => {
      emit({ step: 'prepare', message: `Checking the folder ${folderOf(name, p.settings)}`, percent: 2 })
      await sleep(300)
      if (dbStopped) {
        p.lastFailed = { at: nowS(), why: 'the database server is not running' }
        throw dbProblem()
      }
      emit({ step: 'database', message: 'Copying the database', percent: 20 })
      await sleep(500)
      emit({ step: 'files', message: 'Writing the backup file', percent: 55 })
      await sleep(500)
      if (pass) emit({ step: 'seal', message: 'Locking the file with the password', percent: 85 })
      await sleep(300)
      const at = nowS() + n++ // distinct ids when two are made in one second
      const created = info(name, at, { encrypted: Boolean(pass), label: b.for_move ? 'move' : 'manual', stopped: Boolean(b.stop_server), in_full: b.everything ? 40 : 2, by_reference: b.everything ? 0 : 38, bytes: b.everything ? 310_000_000 : 48_000_000 })
      p.backups.push(created)
      p.lastFailed = null
      emit({ step: 'done', message: 'Backup finished', percent: 100 })
      return { info: created, path: `${folderOf(name, p.settings)}/${created.file}`, notes: b.stop_server ? [] : ['The server was running while the files were copied. The database is one consistent snapshot; the other files were copied one by one, so two of them can be from slightly different moments.'], pruned: [] } satisfies T.BackupCreated
    })
  })
  route('PUT', '/servers/{s}/backups/settings', (m, b) => {
    const p = get(decodeURIComponent(m[1]))
    const s = b as unknown as T.BackupSettings
    if (s.folder && !s.folder.startsWith('/') && !/^[A-Za-z]:\\/.test(s.folder)) throw new Fail(422, 'backup.destination_unwritable', 'Give the full path of the folder, starting from the top (for example /mnt/backups or D:\\Backups).')
    if (s.schedule.cadence === 'weekly' && s.schedule.weekday == null) throw new Fail(422, 'failed', 'Pick a day of the week for a weekly backup.')
    p.settings = s
    return s
  })
  route('POST', '/servers/{s}/backups/prune', (m) => {
    const p = get(decodeURIComponent(m[1]))
    const keep = p.settings.retention.keep_last
    const sorted = [...p.backups].sort((a, b) => b.created_unix - a.created_unix)
    const gone = keep > 0 ? sorted.slice(keep).filter((b) => b.label !== 'move' && b.id !== p.undo) : []
    p.backups = p.backups.filter((b) => !gone.includes(b))
    return { removed: gone.map((b) => b.id) } satisfies T.BackupsRemoved
  })
  route('POST', '/servers/{s}/backups/undo', (m) => {
    const name = decodeURIComponent(m[1])
    const p = get(name)
    if (!p.undo) throw new Fail(422, 'backup.no_undo', 'There is no restore to undo for this server.')
    return startJob('backup_undo', name, async (emit) => {
      emit({ step: 'files', message: 'Putting the safety copy back', percent: 60 })
      await sleep(600)
      p.undo = null
      return { safety_backup: null, database_restored: true, files_restored: 6, resources_restored: [], problems: [], notes: [], restarted: false } satisfies T.RestoreReport
    })
  })
  route('POST', '/servers/{s}/backups/{id}/verify', (m, b) => {
    const name = decodeURIComponent(m[1])
    const bk = findB(name, decodeURIComponent(m[2]))
    return startJob('backup_verify', name, async (emit) => {
      passFor(bk, b.passphrase)
      emit({ step: 'verify', message: 'Opening it and checking every file', percent: 30 })
      await sleep(500)
      return { ok: true, files: 212, bytes: 61_000_000, summary: 'All 212 files match their checksums.' } satisfies T.VerifyResult
    })
  })
  route('POST', '/servers/{s}/backups/{id}/preview', (m, b) => {
    const name = decodeURIComponent(m[1])
    const bk = findB(name, decodeURIComponent(m[2]))
    const running = servers().find((s) => s.name === name)?.status === 'running'
    if (bk.encrypted && !b.passphrase) return { id: bk.id, created_unix: bk.created_unix, encrypted: true, needs_passphrase: true, server: null, database: null, resources: [], stopped: false, server_running: running, will_change: [], will_keep: [], warnings: [] } satisfies T.RestorePreview
    passFor(bk, b.passphrase)
    return {
      id: bk.id,
      created_unix: bk.created_unix,
      encrypted: bk.encrypted,
      needs_passphrase: false,
      server: { name, mode: 'development', source: 'fork', versions: 'latest', game_port: 30120, txadmin_port: 40120, game_build: 3000, artifact_build: 1234, os: 'linux', dir: `/srv/${name}` },
      database: { kind: 'dumped', name: 'kora_ab12cd', host: '127.0.0.1', port: 3306, dump_bytes: 9_000_000, note: '' },
      resources: [],
      stopped: bk.stopped,
      server_running: running,
      will_change: [
        "The server's settings: server.cfg, permissions and the other .cfg files",
        "txAdmin's data (its settings, admins and player records)",
        'The database "kora_ab12cd": every table is emptied and replaced by the ones in the backup (anything saved since then is lost, unless you undo)',
        `${bk.by_reference} resources will be fetched again from GitHub, at the exact version saved (this needs internet)`,
        `${bk.in_full} resources will be replaced by the copy in the backup`,
      ],
      will_keep: ["secrets.cfg and database.json stay as they are now (they hold today's licence key and database login)", 'The FXServer program itself (it is not in backups)', 'Logs'],
      warnings: [...(running ? ['The server is running. Kora will stop it for the restore and start it again afterwards.'] : []), 'A safety copy of the server as it is now is made first, so you can undo this.'],
    } satisfies T.RestorePreview
  })
  route('POST', '/servers/{s}/backups/{id}/restore', (m, b) => {
    const name = decodeURIComponent(m[1])
    const p = get(name)
    const bk = findB(name, decodeURIComponent(m[2]))
    return startJob('backup_restore', name, async (emit) => {
      passFor(bk, b.passphrase)
      emit({ step: 'unpack', message: 'Unpacking and checking every file (nothing is changed yet)', percent: 10 })
      await sleep(500)
      emit({ step: 'safety', message: 'Making a safety copy of the server as it is now', percent: 30 })
      await sleep(500)
      const safety = info(name, nowS() + n++, { label: 'before_restore', encrypted: bk.encrypted })
      p.backups.push(safety)
      p.undo = safety.id
      if (dbStopped) throw dbProblem()
      emit({ step: 'database', message: 'Putting the database back', percent: 55 })
      await sleep(500)
      emit({ step: 'resources', message: 'Bringing the resources back', percent: 80 })
      await sleep(500)
      return { safety_backup: safety.id, database_restored: true, files_restored: 14, resources_restored: ['kora_core is back at version 4f2a9c1e', 'kora_hud is back at version 90bd3e77'], problems: [], notes: ["secrets.cfg was kept as it is now (it holds today's licence key and database login)."], restarted: true } satisfies T.RestoreReport
    })
  })
  // The archive itself is binary: the demo has no files, so it answers like a missing one.
  route('GET', '/servers/{s}/backups/{id}/file', (m) => {
    findB(decodeURIComponent(m[1]), decodeURIComponent(m[2]))
    throw new Fail(404, 'backup.not_found', 'The demo has no backup files to download.')
  })
  route('DELETE', '/servers/{s}/backups/{id}', (m) => {
    const p = get(decodeURIComponent(m[1]))
    const id = decodeURIComponent(m[2])
    findB(decodeURIComponent(m[1]), id)
    p.backups = p.backups.filter((b) => b.id !== id)
    if (p.undo === id) p.undo = null
    return undefined
  })

  // ---- bringing a server in from a backup file
  const uploads = new Map<string, string>()
  const taken = () => servers().map((x) => x.name)
  const previewOf = (name: string, encrypted: boolean, pw: unknown): T.ImportPreview => {
    const base = { file_name: name, encrypted, created_unix: nowS() - 3600, by_reference: 38, in_full: 2, saved_keys: ['Main', 'Spare'], licence_note: 'Your licence key moves with the server, but it is yours, not Kora\'s: Cfx.re ties a key to your account and the address the server runs on. Kora cannot make Cfx.re accept it on a new computer. If the server refuses to start with a licence error, check the key at portal.cfx.re (Keymaster), change its server IP address there if it has one, or pick a different saved key.' }
    if (encrypted && !pw) return { ...base, needs_passphrase: true, server: null, database: null, suggested: null, clashes: [], will_create: [], wont_carry: [], warnings: [], fork: null, db_login: null }
    if (encrypted && pw !== PASSWORD) throw new Fail(422, 'backup.wrong_passphrase', 'That password does not open this backup. Check it and try again. (Kora does not keep passwords, so a lost one cannot be recovered.)')
    void adminGiven
    const used = taken().includes('Old RP server')
    const clashes: T.Clash[] = used ? [{ kind: 'name', message: 'A server called "Old RP server" is already here.', suggestion: 'Old RP server 2' }] : []
    clashes.push({ kind: 'game_port', message: 'The game port 30120 is already used on this computer.', suggestion: '30121' })
    return {
      ...base,
      needs_passphrase: false,
      server: { name: 'Old RP server', mode: 'development', source: 'fork', versions: 'latest', game_port: 30120, txadmin_port: 40120, game_build: 3258, artifact_build: 17000, os: 'windows', dir: 'C:\\Servers\\old-rp', github_owner: 'old-owner' },
      database: { kind: 'dumped', name: 'kora_ab12', host: '127.0.0.1', port: 3306, dump_bytes: 9_000_000, note: '' },
      suggested: { name: used ? 'Old RP server 2' : 'Old RP server', dir: '/home/demo/kora/old-rp-server', game_port: 30121, txadmin_port: 40120 },
      clashes,
      will_create: ['A new server "Old RP server" in /home/demo/kora/old-rp-server', 'Its settings and txAdmin data, with the folders and line endings converted for this computer (the backup comes from Windows)', 'A NEW database and login on this computer\'s database server, filled from the backup (9.0 MB)', '38 resources fetched again from GitHub at the saved versions, 2 taken from the file', 'FXServer build 17000 (downloaded again)'],
      wont_carry: ['The operating-system service that starts the server with the computer (set it up again on this computer).', 'Firewall rules and port forwarding (open the ports again for the new computer).', 'The automatic-update timer and the backup schedule.', 'Your GitHub sign-in (sign in again on this computer if the resources need it).', 'The FXServer program itself: it is downloaded again.'],
      fork: { owner: 'old-owner', usable: false, signed_in_as: 'demo', message: 'This server follows copies (forks) in the GitHub account old-owner, which the account signed in here (demo) cannot use. Choose what to do about them below.' },
      db_login: dbLoginOk ? { ok: true, message: "Kora can log in to this computer's database server automatically." } : { ok: false, message: 'Kora could not log in to the database server as root (access denied).' },
      warnings: ['This server was made on a different operating system. Folders in its settings are rewritten, but a resource that runs a program only one system has (a Linux shell script, a Windows .exe) will not work here.'],
    }
  }
  route('GET', '/backups/archived', () => ({ servers: [{ name: 'Old RP', deleted_unix: nowS() - 5 * DAY, backups: [info('Old RP', nowS() - 6 * DAY, { encrypted: true })] }] }) satisfies T.ArchivedBackups)
  route('POST', '/backups/import/upload', (_m, b) => {
    if (!(b.file instanceof Blob)) throw new Fail(400, 'bad_request', 'Send the backup file itself.')
    const id = `${'ab'.repeat(8)}${n++}`.slice(0, 20).padEnd(20, '0')
    const name = String(b.fileName ?? 'backup.tar.gz.age')
    uploads.set(id, name)
    return { upload: id, file_name: name, bytes: (b.file as Blob).size } satisfies T.ImportUploaded
  })
  route('POST', '/backups/import/inspect', (_m, b) => {
    const src = b.source as T.ImportSource
    const name = src.archived ? `${src.archived.name}-${src.archived.backup}.tar.gz.age` : src.upload ? uploads.get(src.upload) : src.path ? src.path.split(/[\\/]/).pop() : undefined
    if (!name) throw new Fail(404, 'backup.not_found', 'That upload is gone. Upload the file again.')
    if (name.toLowerCase().includes('broken')) throw new Fail(422, 'backup.corrupt', 'This backup cannot be used: the backup file is damaged or incomplete.')
    return previewOf(name, name.endsWith('.age'), b.passphrase)
  })
  route('POST', '/backups/import', (_m, b) => {
    const req = b as unknown as T.ImportRequest
    if (!dbLoginOk && !req.db_admin) throw new Fail(422, 'backup.db_stopped', 'Kora could not create the new database. Is the database server running? Kora uses the same database administrator login as an install: on Linux the root account\'s own login (run Kora as root for the import) or the KORA_DB_ADMIN_USER and KORA_DB_ADMIN_PASSWORD settings; on Windows the login of the MariaDB Kora set up.')
    adminGiven = Boolean(req.db_admin)
    return startJob('backup_import_start', req.choices.name, async (emit) => {
      emit({ step: 'unpack', message: 'Unpacking and checking every file (nothing is placed yet)', percent: 6 })
      await sleep(400)
      emit({ step: 'files', message: 'Placing the settings and txAdmin data', percent: 25 })
      await sleep(400)
      emit({ step: 'artifact', message: 'Getting the server program', percent: 40 })
      await sleep(400)
      if (dbStopped) throw dbProblem()
      emit({ step: 'database', message: 'Creating a new database and login', percent: 55 })
      await sleep(400)
      emit({ step: 'resources', message: 'Fetching kora_core (1/38)', percent: 75 })
      await sleep(400)
      return { name: req.choices.name, dir: req.choices.dir, resources_fetched: ['kora_core is back at version 4f2a9c1e'], problems: [], notes: [], started: req.start, next_steps: ['Open the server\'s ports on this computer (the Overview\'s "Can players reach my server?" shows how).', 'If you want it to start with the computer, set up the background service.', 'Check your licence key at portal.cfx.re if the server says it is not valid here.'], old_machine_steps: [`When you are sure "${req.choices.name}" works here, stop it on the OLD computer (open Kora there, Servers, Stop).`, "Then remove it there: open the server's Danger zone tab and choose Remove. Kora never does this for you from here."] } satisfies T.ImportResult
    })
  })

  return {
    reset(scenario: string) {
      uploads.clear()
      per = {}
      n = 1
      dbStopped = scenario === 'trouble'
      dbLoginOk = scenario !== 'windows'
      adminGiven = false
      const all = servers()
      const first = all[0]
      if (first && scenario !== 'empty') {
        const p = get(first.name)
        const t = nowS()
        p.backups = [info(first.name, t - 3 * DAY, { label: 'scheduled' }), info(first.name, t - 2 * DAY, { label: 'scheduled' }), info(first.name, t - 5 * 3600, { encrypted: true })]
        if (scenario === 'fleet') p.settings.schedule.enabled = true
      }
      if (scenario === 'fleet') for (const s of all.slice(1, 3)) get(s.name).backups = [info(s.name, nowS() - 4 * DAY)]
    },
  }
}

export const MOCK_BACKUP_PASSWORD = PASSWORD
