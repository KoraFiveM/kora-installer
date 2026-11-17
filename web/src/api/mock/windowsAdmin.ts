// Mock of the Windows administrator worker (`kora_web::elevated`, `kora_core::helper::worker`): the
// panel asks Windows for permission itself, once per install. Demo install names: "decline..." (the
// person chooses No), "nobody..." (nobody answers), "remote..." (signed in from another device: Windows
// only asks when the person says so). Asking again, or continuing without, then works.
import type * as T from '../types'

export const ADMIN_STEP = 'admin'

/** What the install needs administrator rights for (as the real plan words it). */
export function adminNeeds(r: T.InstallRequest): string[] {
  const needs: string[] = []
  if (r.firewall) needs.push('Open the game port in the firewall')
  if (r.txadmin_public) needs.push('Open the txAdmin port in the firewall')
  if (r.service) needs.push('Register the server as a Windows service')
  if (r.db === 'install_if_missing') needs.push('Install the database server (MariaDB) as a Windows service')
  return needs
}

/** "Continue without the administrator parts" (the same rule as the wizard and the panel). */
export function withoutAdminReq(r: T.InstallRequest): T.InstallRequest {
  return { ...r, firewall: false, txadmin_public: false, service: false, db: r.db === 'install_if_missing' ? 'use_existing' : r.db }
}

export type PromptAnswer = 'yes' | 'no' | 'nobody'

/** How the mock prompt is answered: by the install's name, the first time only. */
export function promptAnswer(name: string, attempt: number): PromptAnswer {
  const n = name.toLowerCase()
  if (attempt > 0) return 'yes'
  if (n.startsWith('decline')) return 'no'
  if (n.startsWith('nobody')) return 'nobody'
  return 'yes'
}

export function installCommand(r: T.InstallRequest): string[] {
  return ['kora', 'install', '--name', r.name, '--dir', r.dir, ...(r.service ? ['--service'] : []), ...(r.firewall ? ['--firewall'] : []), '--yes']
}

const what = (r: T.InstallRequest) => adminNeeds(r).map((n) => n[0].toLowerCase() + n.slice(1)).join(' and ')

/** The choices every failed or skipped prompt offers. */
function choices(r: T.InstallRequest): T.ProblemAction[] {
  return [{ kind: 'request_admin' }, { kind: 'continue_without_admin' }, { kind: 'show_command', command: installCommand(r), elevated: true }]
}

export function promptProblem(r: T.InstallRequest, a: Exclude<PromptAnswer, 'yes'>): Pick<T.JobError, 'code' | 'message' | 'details' | 'actions'> {
  if (a === 'no') {
    return { code: 'admin.declined', message: `Windows did not get permission to ${what(r)}, so nothing that needs administrator rights was changed.`, details: 'The Windows permission prompt was answered with No (or closed).', actions: choices(r) }
  }
  return {
    code: 'admin.timed_out',
    message: `Nobody answered the Windows permission prompt in time, so Kora could not ${what(r)}. Nothing that needs administrator rights was changed.`,
    details: 'The prompt appears on the computer that runs Kora. If you did not see it, look for a flashing shield icon in the taskbar there.',
    actions: choices(r),
  }
}

export function remoteProblem(r: T.InstallRequest): Pick<T.JobError, 'code' | 'message' | 'actions'> {
  return {
    code: 'admin.remote',
    message: `To ${what(r)}, Windows has to ask for administrator permission on the computer that runs Kora. You are signed in from another device, so nobody may be there to answer. If someone is at that computer, ask for permission and they can choose Yes.`,
    actions: choices(r),
  }
}
