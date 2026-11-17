// Types and calls for the job history and the audit log (the store workstream).
// Kept in their own file for now: `types.ts` and `client.ts` are being replaced by generated code, and these
// two endpoints (`GET /jobs`, `GET /audit`) will come from that generator once it knows them.
import { request } from './client'
import type { JobError } from './types'

export type AuditResult = 'ok' | 'failed' | 'refused' | 'started'

export interface AuditRow {
  id: number
  /** Unix seconds. */
  at: number
  /** The operation's name (`server_control`, `key_add`, ...). */
  op: string
  server: string | null
  /** `local` (the person on this computer), `remote` (signed in from another device) or `cli` (a `kora` command). */
  actor_kind: 'local' | 'remote' | 'cli'
  actor_name: string | null
  /** For remote: `token` or `tailscale`. */
  provider: string | null
  role: string | null
  result: AuditResult
  /** A short note Kora wrote (a job id, an error code). Never a secret. */
  detail: string | null
}

export interface AuditPage {
  entries: AuditRow[]
  more: boolean
  retention_days: number
}

export interface AuditQuery {
  limit?: number
  /** Only rows older than this id (to show more). */
  before?: number
  server?: string
  op?: string
  actor?: string
  result?: string
}

/** A job as the history lists it. `interrupted`: the panel stopped while it was running. */
export type HistoryState = 'running' | 'done' | 'failed' | 'interrupted'

export interface JobSummary {
  id: string
  op: string
  server: string | null
  state: HistoryState
  /** Unix seconds. */
  started: number
  ended: number | null
  result: unknown
  error: JobError | null
}

export const storeApi = {
  audit: (q: AuditQuery = {}) => request<AuditPage>('/audit', { query: { limit: q.limit, before: q.before, server: q.server || undefined, op: q.op || undefined, actor: q.actor || undefined, result: q.result || undefined } }),
  jobs: (limit = 50) => request<{ jobs: JobSummary[] }>('/jobs', { query: { limit } }),
}

/** Plain words for an operation name; anything unknown reads as its name with spaces. */
const LABELS: Record<string, string> = {
  server_control: 'Started, stopped or restarted a server',
  server_delete: 'Deleted a server',
  server_unregister: 'Removed a server from Kora',
  servers_adopt: 'Added an existing server',
  update_apply: 'Updated resources',
  update_auto: 'Updated resources (scheduled)',
  snapshot_rollback: 'Rolled back an update',
  resources_add: 'Added resources',
  resource_ensure: 'Started a resource',
  resource_control: 'Controlled a resource',
  console_send: 'Sent a console command',
  console_enable: 'Changed the console setting',
  webhook_set: 'Set the alert webhook',
  webhook_remove: 'Removed the alert webhook',
  schedule_set: 'Set the update schedule',
  schedule_remove: 'Turned off the update schedule',
  ports_set: 'Changed ports',
  install_start: 'Installed a server',
  install_resume: 'Carried on with an install',
  install_saved_clear: 'Forgot the saved install answers',
  key_add: 'Saved a licence key',
  key_update: 'Changed a saved licence key',
  key_remove: 'Forgot a saved licence key',
  server_key_set: "Changed a server's licence key",
  profile_set: 'Changed the remembered names',
  user_add: 'Added a remote user',
  user_remove: 'Removed a remote user',
  remote_set: 'Changed remote access',
  node_add: 'Added a node',
  node_remove: 'Removed a node',
  node_server_control: 'Controlled a server on a node',
  self_update_apply: 'Updated Kora',
  app_install: 'Installed the Kora app',
  app_uninstall: 'Uninstalled the Kora app',
  uninstall_run: 'Uninstalled Kora',
  asset_import: 'Imported a paid asset',
  asset_remove: 'Removed a paid asset',
  tool_install: 'Installed a tool',
  db_start: 'Started the database',
  branch_switch: 'Switched a branch',
  branch_bulk_apply: 'Switched many branches',
  stash_restore: 'Brought back kept changes',
  alert_settings_set: 'Changed alert settings',
  panel_restart: 'Restarted the panel',
  session_link: 'Made a sign-in link',
  github_login: 'Signed in to GitHub',
  issue_report: 'Reported an issue',
  pr_open: 'Opened a pull request',
}

export function opLabel(op: string): string {
  return LABELS[op] ?? op.replace(/_/g, ' ').replace(/^./, (c) => c.toUpperCase())
}

/** Who did it, in words. */
export function whoLabel(r: Pick<AuditRow, 'actor_kind' | 'actor_name' | 'provider' | 'role'>): string {
  const n = r.actor_name ?? 'someone'
  if (r.actor_kind === 'remote') return `${n} (signed in from another device${r.provider ? ` with ${r.provider}` : ''}${r.role ? `, ${r.role}` : ''})`
  if (r.actor_kind === 'cli') return `${n} (a kora command)`
  return `${n} (on this computer)`
}
