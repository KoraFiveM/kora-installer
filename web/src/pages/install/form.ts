import type { DbChoice, InstallDefaults, InstallRequest, Mode, SavedInstall } from '@/api/types'
import { joinPath, slug, validatePorts } from '@/lib/utils'

export type ArtifactChoice = 'recommended' | 'pick' | 'custom'

export interface WizardForm {
  mode: Mode
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
  app: boolean
  desktopShortcut: boolean
  license: string
  githubOwner: string
}

export type StepId = 'mode' | 'name' | 'ports' | 'build' | 'resources' | 'database' | 'options' | 'licence' | 'github' | 'review'

export const STEP_LABEL: Record<StepId, string> = {
  mode: 'Mode',
  name: 'Name and folder',
  ports: 'Ports',
  build: 'FiveM server build',
  resources: 'Resources',
  database: 'Database',
  options: 'Options',
  licence: 'Licence key',
  github: 'GitHub',
  review: 'Review',
}

export function stepsFor(mode: Mode): StepId[] {
  const s: StepId[] = ['mode', 'name', 'ports', 'build', 'resources', 'database', 'options', 'licence']
  if (mode === 'development') s.push('github')
  s.push('review')
  return s
}

export function dirBase(dir: string): string {
  const i = Math.max(dir.lastIndexOf('/'), dir.lastIndexOf('\\'))
  return i > 0 ? dir.slice(0, i) : dir
}

export function initialForm(d: InstallDefaults): WizardForm {
  return {
    mode: 'production', name: d.name, dir: d.dir, dirTouched: false, hostname: d.hostname,
    game: String(d.ports.game), tx: String(d.ports.txadmin),
    artifact: 'recommended', artifactBuild: null, artifactUrl: '', gameBuild: '',
    resourcesAll: true, resourceIds: [],
    db: 'use_existing', dbCustom: false, dbUser: '', dbPass: '', dbPass2: '', dbHost: '127.0.0.1', dbPort: '3306',
    firewall: true, txPublic: false, service: true, app: true, desktopShortcut: true,
    license: '', githubOwner: '',
  }
}

/** Fill the form from answers saved by an install that stopped part-way (never contains secrets). */
export function applySaved(f: WizardForm, s: SavedInstall): WizardForm {
  return {
    ...f,
    mode: s.mode ?? f.mode,
    name: s.name ?? f.name,
    dir: s.dir ?? f.dir,
    dirTouched: s.dir ? true : f.dirTouched,
    hostname: s.hostname ?? f.hostname,
    game: s.ports ? String(s.ports.game) : f.game,
    tx: s.ports ? String(s.ports.txadmin) : f.tx,
    resourcesAll: s.resources ? s.resources === 'all' : f.resourcesAll,
    resourceIds: s.resources && s.resources !== 'all' ? s.resources.only : f.resourceIds,
    db: s.db ?? f.db,
    service: s.service ?? f.service,
    firewall: s.firewall ?? f.firewall,
    txPublic: s.txadmin_public ?? f.txPublic,
    githubOwner: s.github_owner ?? f.githubOwner,
  }
}

export function folderFor(base: string, name: string): string {
  return joinPath(base, slug(name) || 'my-server')
}

export function buildRequest(f: WizardForm, opts: { appSupported?: boolean; artifactUrl?: string | null } = {}): InstallRequest {
  const gb = f.gameBuild.trim()
  return {
    mode: f.mode,
    name: slug(f.name),
    dir: f.dir.trim(),
    hostname: f.hostname.trim(),
    license_key: f.license.trim() || null,
    ports: { game: Number(f.game), txadmin: Number(f.tx) },
    txadmin_public: f.txPublic,
    artifact_url: f.artifact === 'custom' ? f.artifactUrl.trim() : f.artifact === 'pick' ? (opts.artifactUrl ?? null) : null,
    game_build: gb ? Number(gb) : null,
    resources: f.resourcesAll ? 'all' : { only: f.resourceIds },
    db: f.db,
    service: f.service,
    firewall: f.firewall,
    github_owner: f.mode === 'development' ? f.githubOwner || null : null,
    dry_run: false,
    db_admin: f.db !== 'skip' && f.dbCustom ? { host: f.dbHost.trim(), port: Number(f.dbPort), user: f.dbUser.trim(), password: f.dbPass } : null,
    start: true,
    app: opts.appSupported && f.app ? { desktop_shortcut: f.desktopShortcut } : null,
  }
}

export function dbLoginProblem(f: WizardForm): string | null {
  if (f.db === 'skip' || !f.dbCustom) return null
  if (!f.dbUser.trim()) return 'Type the database administrator user name.'
  if (!f.dbPass) return 'Type the database administrator password.'
  if (f.dbPass !== f.dbPass2) return 'The two passwords are not the same.'
  const p = Number(f.dbPort)
  if (!Number.isInteger(p) || p < 1 || p > 65535) return 'The database port must be a number from 1 to 65535.'
  if (!f.dbHost.trim()) return 'Type the database host.'
  return null
}

/** Why the Next button is disabled for a step (null = fine). Messages are shown to the user. */
export function stepProblem(step: StepId, f: WizardForm, ctx: { portsOk: boolean; githubSignedIn: boolean }): string | null {
  switch (step) {
    case 'name':
      if (!slug(f.name)) return 'Give the server a name.'
      if (!f.dir.trim()) return 'Choose a folder for the server.'
      return null
    case 'ports':
      return validatePorts(f.game, f.tx) ?? (ctx.portsOk ? null : 'One of the ports is already in use. Pick a free one.')
    case 'build':
      if (f.artifact === 'custom' && !/^https:\/\/\S+$/.test(f.artifactUrl.trim())) return 'Type a download address that starts with https://.'
      if (f.artifact === 'pick' && f.artifactBuild === null) return 'Pick a build from the list.'
      if (f.gameBuild.trim() && !/^\d+$/.test(f.gameBuild.trim())) return 'The game build must be a number.'
      return null
    case 'resources':
      return !f.resourcesAll && f.resourceIds.length === 0 ? 'Pick at least one resource, or choose to install everything.' : null
    case 'database':
      return dbLoginProblem(f)
    case 'licence':
      return f.license.trim().length < 8 ? 'Paste your licence key. It is a long string of letters and numbers.' : null
    case 'github':
      if (!ctx.githubSignedIn) return 'Sign in to GitHub to continue.'
      return f.githubOwner ? null : 'Choose where your forks go.'
    default:
      return null
  }
}
