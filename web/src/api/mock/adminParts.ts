// Mock helpers for the administrator service (`kora_web::elevated`, `kora_core::privsvc`): what an install
// needs administrator rights for, and the install with those parts left out.
import type * as T from '../types'

/** What the install needs administrator rights for (as the real plan words it). */
export function adminNeeds(r: T.InstallRequest): string[] {
  const needs: string[] = []
  if (r.firewall) needs.push('Open the game port in the firewall')
  if (r.txadmin_public) needs.push('Open the txAdmin port in the firewall')
  if (r.service) needs.push('Register the server as a system service')
  if (r.db === 'install_if_missing') needs.push('Install the database server (MariaDB) as a Windows service')
  return needs
}

/** "Continue without the administrator parts" (the same rule as the wizard and the panel). */
export function withoutAdminReq(r: T.InstallRequest): T.InstallRequest {
  return { ...r, firewall: false, txadmin_public: false, service: false, db: r.db === 'install_if_missing' ? 'use_existing' : r.db }
}

export function installCommand(r: T.InstallRequest): string[] {
  return ['kora', 'install', '--name', r.name, '--dir', r.dir, ...(r.service ? ['--service'] : []), ...(r.firewall ? ['--firewall'] : []), '--yes']
}

/** What an install says it needs administrator rights for, as one sentence ("open the game port in the firewall and ..."). */
export const adminWhat = (r: T.InstallRequest) => adminNeeds(r).map((n) => n[0].toLowerCase() + n.slice(1)).join(' and ')
