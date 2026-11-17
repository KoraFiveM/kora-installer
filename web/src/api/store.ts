// Plain words for the job history and the audit log. The types (`JobSummary`, `AuditRow`, ...) are generated from
// the Rust side (`crates/kora-core/src/api/wire/`); the calls are `api.jobs.list` and `api.audit.list`.
import type { AuditRow } from './types'

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
  backup_create: 'Made a backup',
  backup_restore: 'Restored a backup',
  backup_undo: 'Undid a restore',
  backup_verify: 'Checked a backup',
  backup_delete: 'Deleted a backup',
  backup_settings_set: 'Changed backup settings',
  backup_prune: 'Removed old backups',
  backup_download: 'Downloaded a backup',
  backup_import_start: 'Brought a server in from a backup',
  support_bundle_create: 'Saved a support bundle',
  support_bundle_download: 'Downloaded a support bundle',
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
