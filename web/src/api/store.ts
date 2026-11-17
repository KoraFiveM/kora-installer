// Plain words for the job history and the audit log. The types (`JobSummary`, `AuditRow`, ...) are generated from
// the Rust side (`crates/kora-core/src/api/wire/`); the calls are `api.jobs.list` and `api.audit.list`.
import type { AuditRow } from './types'
import { t, msg, type Msg } from '@/i18n'
import '@/i18n/en/core'
import '@/i18n/en/settings'

/** Plain words for an operation name; anything unknown reads as its name with spaces. */
const LABELS: Record<string, Msg> = {
  server_control: msg('settings.store.startedStoppedRestartedServer'),
  server_delete: msg('settings.store.deletedServer'),
  server_unregister: msg('settings.store.removedServerFromKora'),
  servers_adopt: msg('settings.store.addedExistingServer'),
  update_apply: msg('settings.store.updatedResources'),
  update_auto: msg('settings.store.updatedResourcesScheduled'),
  snapshot_rollback: msg('settings.store.rolledBackUpdate'),
  resources_add: msg('settings.store.addedResources'),
  resource_ensure: msg('settings.store.startedResource'),
  resource_control: msg('settings.store.controlledResource'),
  console_send: msg('settings.store.sentConsoleCommand'),
  console_enable: msg('settings.store.changedConsoleSetting'),
  webhook_set: msg('settings.store.setAlertWebhook'),
  webhook_remove: msg('settings.store.removedAlertWebhook'),
  schedule_set: msg('settings.store.setUpdateSchedule'),
  schedule_remove: msg('settings.store.turnedOffUpdateSchedule'),
  ports_set: msg('settings.store.changedPorts'),
  install_start: msg('settings.store.installedServer'),
  install_resume: msg('settings.store.carriedInstall'),
  install_saved_clear: msg('settings.store.forgotSavedInstallAnswers'),
  key_add: msg('settings.store.savedLicenceKey'),
  key_update: msg('settings.store.changedSavedLicenceKey'),
  key_remove: msg('settings.store.forgotSavedLicenceKey'),
  server_key_set: msg('settings.store.changedServersLicenceKey'),
  profile_set: msg('settings.store.changedRememberedNames'),
  user_add: msg('settings.store.addedRemoteUser'),
  user_remove: msg('settings.store.removedRemoteUser'),
  remote_set: msg('settings.store.changedRemoteAccess'),
  node_add: msg('settings.store.addedNode'),
  node_remove: msg('settings.store.removedNode'),
  node_server_control: msg('settings.store.controlledServerNode'),
  self_update_apply: msg('settings.store.updatedKora'),
  app_install: msg('settings.store.installedKoraApp'),
  app_uninstall: msg('settings.store.uninstalledKoraApp'),
  uninstall_run: msg('settings.store.uninstalledKora'),
  asset_import: msg('settings.store.importedPaidAsset'),
  asset_remove: msg('settings.store.removedPaidAsset'),
  tool_install: msg('settings.store.installedTool'),
  db_start: msg('settings.store.startedDatabase'),
  branch_switch: msg('settings.store.switchedBranch'),
  branch_bulk_apply: msg('settings.store.switchedManyBranches'),
  stash_restore: msg('settings.store.broughtBackKeptChanges'),
  alert_settings_set: msg('settings.store.changedAlertSettings'),
  panel_restart: msg('settings.store.restartedPanel'),
  session_link: msg('settings.store.madeSigninLink'),
  github_login: msg('settings.store.signedGithub'),
  issue_report: msg('settings.store.reportedIssue'),
  pr_open: msg('settings.store.openedPullRequest'),
  backup_create: msg('settings.store.madeBackup'),
  backup_restore: msg('settings.store.restoredBackup'),
  backup_undo: msg('settings.store.undidRestore'),
  backup_verify: msg('settings.store.checkedBackup'),
  backup_delete: msg('settings.store.deletedBackup'),
  backup_settings_set: msg('settings.store.changedBackupSettings'),
  backup_prune: msg('settings.store.removedOldBackups'),
  backup_download: msg('settings.store.downloadedBackup'),
  backup_import_start: msg('settings.store.broughtServerFromBackup'),
  support_bundle_create: msg('settings.store.savedSupportBundle'),
  support_bundle_download: msg('settings.store.downloadedSupportBundle'),
}

export function opLabel(op: string): string {
  return LABELS[op] ? t(LABELS[op]) : op.replace(/_/g, ' ').replace(/^./, (c) => c.toUpperCase())
}

/** Who did it, in words. */
export function whoLabel(r: Pick<AuditRow, 'actor_kind' | 'actor_name' | 'provider' | 'role'>): string {
  const n = r.actor_name ?? t('core.store.someone')
  if (r.actor_kind === 'remote') return t('core.store.nSignedFromAnother', { n, kind: r.provider && r.role ? 'both' : r.provider ? 'provider' : r.role ? 'role' : 'none', provider: r.provider ?? '', role: r.role ?? '' })
  if (r.actor_kind === 'cli') return t('core.store.nKoraCommand', { n })
  return t('core.store.nComputer', { n })
}
