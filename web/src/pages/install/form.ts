import type { DbChoice, InstallDefaults, InstallRequest, Mode, RunMode, SavedInstall, SavedKey, Source, Versions } from '@/api/types'
import { joinPath, slug, validatePorts } from '@/lib/utils'

export type ArtifactChoice = 'recommended' | 'pick' | 'custom'

/**
 * The old `mode` as the two settings (production = clones of released versions, development = your own copies of the
 * latest changes). The one place the web reads that old spelling, for answers saved by an older Kora.
 */
export function styleOfMode(m: Mode): { source: Source; versions: Versions } {
  return m === 'development' ? { source: 'fork', versions: 'latest' } : { source: 'clone', versions: 'released' }
}

/** What "Which versions" starts as for a source: released for plain clones, the latest changes for your own copies. */
export function defaultVersions(source: Source): Versions {
  return source === 'fork' ? 'latest' : 'released'
}

export interface WizardForm {
  /** Where the resources come from: plain clones, or the person's own copies (forks) on GitHub. */
  source: Source
  /** Which versions to follow. Follows `source` until the person picks one themselves. */
  versions: Versions
  versionsTouched: boolean
  name: string
  dir: string
  dirTouched: boolean
  hostname: string
  game: string
  tx: string
  artifact: ArtifactChoice
  artifactBuild: number | null
  artifactUrl: string
  gameBuild: string
  resourcesAll: boolean
  resourceIds: string[]
  db: DbChoice
  dbCustom: boolean
  dbUser: string
  dbPass: string
  dbPass2: string
  dbHost: string
  dbPort: string
  firewall: boolean
  txPublic: boolean
  service: boolean
  /** How the server runs: as a system service (default), or in a container (Linux, when a container engine works). */
  runMode: RunMode
  /** The licence key page: pick one of the saved keys, or paste a new one. */
  licenceChoice: 'saved' | 'new'
  /** The saved key picked (`licenceChoice` is `saved`). */
  licenceName: string
  /** A pasted key (`licenceChoice` is `new`). */
  license: string
  /** Keep a pasted key on this computer for next time (on unless the person turns it off). */
  saveKey: boolean
  saveKeyName: string
  githubOwner: string
  /** The GitHub owner came from the last install (shown with a hint to change it). */
  githubRemembered: boolean
  /** The Cfx.re account came from the last install. */
  cfxRemembered: boolean
  /** What the person typed in the Cfx.re step. */
  cfxUsername: string
  /** The account the last successful check found for `cfxUsername` (null = not checked / skipped). `username` is null when restored from saved answers. */
  cfxFound: { username: string | null; identifier: string } | null
}

export type StepId = 'source' | 'versions' | 'name' | 'ports' | 'build' | 'resources' | 'runmode' | 'database' | 'options' | 'licence' | 'cfx' | 'github' | 'review'

export const STEP_LABEL: Record<StepId, string> = {
  source: 'Where the resources come from',
  versions: 'Which versions',
  name: 'Name your server',
  ports: 'Ports',
  build: 'FiveM server build',
  resources: 'Resources',
  runmode: 'How the server runs',
  database: 'Database',
  options: 'Options',
  licence: 'Your licence key',
  cfx: 'Your Cfx.re account',
  github: 'GitHub',
  review: 'Check and install',
}

/** Every step, one after the other ("Choose everything myself"). `containers`: a container engine works here, so "How the server runs" is asked. */
export function stepsFor(source: Source, containers = false): StepId[] {
  const s: StepId[] = ['source', 'versions', 'name', 'ports', 'build', 'resources', ...(containers ? (['runmode'] as StepId[]) : []), 'database', 'options', 'licence', 'cfx']
  if (source === 'fork') s.push('github')
  s.push('review')
  return s
}

/**
 * The short path: only what a person must answer. Everything else keeps its default and is listed on
 * the last screen, where each line can be changed. Your own copies cannot be made without GitHub,
 * so that step joins in.
 */
export function quickStepsFor(source: Source): StepId[] {
  return source === 'fork' ? ['licence', 'name', 'github', 'review'] : ['licence', 'name', 'review']
}

export function dirBase(dir: string): string {
  const i = Math.max(dir.lastIndexOf('/'), dir.lastIndexOf('\\'))
  return i > 0 ? dir.slice(0, i) : dir
}

/** The name offered for a key about to be saved: "Main" first, then "Key 2", "Key 3"... */
export function suggestKeyName(keys: SavedKey[]): string {
  if (keys.length === 0) return 'Main'
  for (let n = keys.length + 1; ; n++) {
    const name = `Key ${n}`
    if (!keys.some((k) => k.name.toLowerCase() === name.toLowerCase())) return name
  }
}

export function initialForm(d: InstallDefaults, keys: SavedKey[] = []): WizardForm {
  const cfxId = d.profile?.cfx_identifier ?? null
  return {
    source: 'clone', versions: 'released', versionsTouched: false, name: d.name, dir: d.dir, dirTouched: false, hostname: d.hostname,
    game: String(d.ports.game), tx: String(d.ports.txadmin),
    artifact: 'recommended', artifactBuild: null, artifactUrl: '', gameBuild: '',
    resourcesAll: true, resourceIds: [],
    // Works on a clean machine: uses a database server that is already there, installs MariaDB only when none is found.
    db: 'install_if_missing', dbCustom: false, dbUser: '', dbPass: '', dbPass2: '', dbHost: '127.0.0.1', dbPort: '3306',
    firewall: true, txPublic: false, service: true, runMode: 'service',
    licenceChoice: keys.length > 0 ? 'saved' : 'new', licenceName: keys[0]?.name ?? '', license: '', saveKey: true, saveKeyName: suggestKeyName(keys),
    // What Kora remembers from the last install: still editable on their own pages.
    githubOwner: d.profile?.github_owner ?? '', githubRemembered: !!d.profile?.github_owner,
    cfxUsername: d.profile?.cfx_username ?? '', cfxFound: cfxId ? { username: d.profile?.cfx_username ?? null, identifier: cfxId } : null, cfxRemembered: !!cfxId,
  }
}

/** The saved answers' two settings: the new fields when there are some, else mapped from the old `mode`. */
function resolveSaved(s: SavedInstall, f: WizardForm): Pick<WizardForm, 'source' | 'versions' | 'versionsTouched'> {
  const old = s.mode ? styleOfMode(s.mode) : null
  const source = s.source ?? old?.source ?? f.source
  return { source, versions: s.versions ?? old?.versions ?? defaultVersions(source), versionsTouched: true }
}

/** Fill the form from answers saved by an install that stopped part-way (never contains secrets). */
export function applySaved(f: WizardForm, s: SavedInstall): WizardForm {
  return {
    ...f,
    ...(s.source || s.versions || s.mode ? resolveSaved(s, f) : {}),
    name: s.name ?? f.name,
    dir: s.dir ?? f.dir,
    dirTouched: s.dir ? true : f.dirTouched,
    hostname: s.hostname ?? f.hostname,
    game: s.ports ? String(s.ports.game) : f.game,
    tx: s.ports ? String(s.ports.txadmin) : f.tx,
    resourcesAll: s.resources ? s.resources === 'all' : f.resourcesAll,
    resourceIds: s.resources && s.resources !== 'all' ? s.resources.only : f.resourceIds,
    db: s.db ?? f.db,
    runMode: s.run_mode ?? f.runMode,
    service: s.service ?? f.service,
    firewall: s.firewall ?? f.firewall,
    txPublic: s.txadmin_public ?? f.txPublic,
    githubOwner: s.github_owner ?? f.githubOwner,
    cfxFound: s.cfx_identifier ? { username: null, identifier: s.cfx_identifier } : f.cfxFound,
    licenceChoice: s.license_key_name ? 'saved' : f.licenceChoice,
    licenceName: s.license_key_name ?? f.licenceName,
  }
}

/**
 * The answers without anything that needs administrator rights ("Continue without the administrator
 * parts"): no firewall change, no background service, and the database only used when
 * it is already there.
 */
export function withoutAdmin(f: WizardForm): WizardForm {
  return { ...f, firewall: false, txPublic: false, service: false, db: f.db === 'install_if_missing' ? 'use_existing' : f.db }
}

/** Choosing how the server runs also moves the database to that mode's usual answer (a container next to the server, or what a service installs). */
export function runModePatch(f: WizardForm, runMode: RunMode): Partial<WizardForm> {
  // A container server needs no administrator rights: the firewall step (which does) is off until the person asks for it.
  if (runMode === 'container') return { runMode, db: f.db === 'skip' ? 'skip' : 'container', dbCustom: false, firewall: false }
  return { runMode, db: f.db === 'container' ? 'install_if_missing' : f.db, firewall: true }
}

/** The wizard page a `change_answers` action names, when this kind of server has it. */
export function sectionStep(section: string | undefined, source: Source): StepId | null {
  const all = stepsFor(source, true)
  // The old name of the first page.
  if (section === 'mode') section = 'source'
  if (section === 'run_mode') section = 'runmode'
  return section && (all as string[]).includes(section) && section !== 'review' ? (section as StepId) : null
}

export function folderFor(base: string, name: string): string {
  return joinPath(base, slug(name) || 'my-server')
}

export function buildRequest(f: WizardForm, opts: { artifactUrl?: string | null } = {}): InstallRequest {
  const gb = f.gameBuild.trim()
  return {
    source: f.source,
    versions: f.versions,
    name: slug(f.name),
    dir: f.dir.trim(),
    hostname: f.hostname.trim(),
    license_key: f.licenceChoice === 'new' ? f.license.trim() || null : null,
    license_key_name: f.licenceChoice === 'saved' ? f.licenceName : undefined,
    save_license_key_as: f.licenceChoice === 'new' && f.saveKey && f.license.trim() ? f.saveKeyName.trim() : undefined,
    ports: { game: Number(f.game), txadmin: Number(f.tx) },
    txadmin_public: f.txPublic,
    artifact_url: f.artifact === 'custom' ? f.artifactUrl.trim() : f.artifact === 'pick' ? (opts.artifactUrl ?? null) : null,
    game_build: gb ? Number(gb) : null,
    resources: f.resourcesAll ? 'all' : { only: f.resourceIds },
    db: f.db,
    // A container server has no system service: it restarts through its engine.
    service: f.runMode === 'container' ? false : f.service,
    ...(f.runMode === 'container' ? { run_mode: 'container' as const, container: {} } : {}),
    firewall: f.firewall,
    github_owner: f.source === 'fork' ? f.githubOwner || null : null,
    dry_run: false,
    db_admin: f.db !== 'skip' && f.db !== 'container' && f.dbCustom ? { host: f.dbHost.trim(), port: Number(f.dbPort), user: f.dbUser.trim(), password: f.dbPass } : null,
    start: true,
    cfx_identifier: f.cfxFound?.identifier ?? null,
  }
}

export function dbLoginProblem(f: WizardForm): string | null {
  if (f.db === 'skip' || f.db === 'container' || !f.dbCustom) return null
  if (!f.dbUser.trim()) return 'Type the database administrator user name.'
  if (!f.dbPass) return 'Type the database administrator password.'
  if (f.dbPass !== f.dbPass2) return 'The two passwords are not the same.'
  const p = Number(f.dbPort)
  if (!Number.isInteger(p) || p < 1 || p > 65535) return 'The database port must be a number from 1 to 65535.'
  if (!f.dbHost.trim()) return 'Type the database host.'
  return null
}

/** What Kora knows about the two ports: still asking, both free, one taken, or the check itself failed. */
export type PortsState = 'checking' | 'ok' | 'taken' | 'unknown'

export interface ProblemContext {
  ports: PortsState
  githubSignedIn: boolean
  /** A container engine works here (the System check's Containers line is fine). */
  containers?: boolean
}

const PORTS_MESSAGE: Record<PortsState, string | null> = {
  ok: null,
  checking: 'Checking that the ports are free...',
  taken: 'One of the ports is already in use. Pick a free one.',
  unknown: 'Kora could not check that the ports are free.',
}

/** Why a step is not finished (null = fine). Messages are shown to the user. */
export function stepProblem(step: StepId, f: WizardForm, ctx: ProblemContext): string | null {
  switch (step) {
    case 'name':
      if (!slug(f.name)) return 'Give the server a name.'
      if (!f.dir.trim()) return 'Choose a folder for the server.'
      return null
    case 'ports':
      return validatePorts(f.game, f.tx) ?? PORTS_MESSAGE[ctx.ports]
    case 'build':
      if (f.artifact === 'custom' && !/^https:\/\/\S+$/.test(f.artifactUrl.trim())) return 'Type a download address that starts with https://.'
      if (f.artifact === 'pick' && f.artifactBuild === null) return 'Pick a build from the list.'
      if (f.gameBuild.trim() && !/^\d+$/.test(f.gameBuild.trim())) return 'The game build must be a number.'
      return null
    case 'resources':
      return !f.resourcesAll && f.resourceIds.length === 0 ? 'Pick at least one resource, or choose to install everything.' : null
    case 'runmode':
      return f.runMode === 'container' && !ctx.containers ? 'Running in a container needs Podman or Docker, and none that works was found. Choose a system service, or fix the container engine on the System check page.' : null
    case 'database':
      return dbLoginProblem(f)
    case 'licence':
      if (f.licenceChoice === 'saved') return f.licenceName ? null : 'Pick one of your saved keys, or add a new one.'
      if (f.license.trim().length < 8) return 'Paste your licence key. It is a long string of letters and numbers.'
      return f.saveKey && !f.saveKeyName.trim() ? 'Give the key a name so you can find it next time, or turn off "Save this key".' : null
    case 'cfx':
      return f.cfxFound || !f.cfxUsername.trim() ? null : 'Check the Cfx.re username, or clear the box to skip this step.'
    case 'github':
      if (!ctx.githubSignedIn) return 'Sign in to GitHub to continue.'
      return f.githubOwner ? null : 'Choose where your own copies go.'
    default:
      return null
  }
}

/** Every step that still has a problem, whether or not the person opened it. Used by the last screen. */
export function allProblems(f: WizardForm, ctx: ProblemContext): Partial<Record<StepId, string>> {
  const out: Partial<Record<StepId, string>> = {}
  for (const id of stepsFor(f.source, true)) {
    const p = stepProblem(id, f, ctx)
    if (p) out[id] = p
  }
  return out
}
