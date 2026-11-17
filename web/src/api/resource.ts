// API types and calls for the resource page. Kept in their own file; the generated client does not know them yet.
import { enc, request } from './client'
import type { ResourceBranch } from './types'

/** Is the running server still on an old version of this resource? Kept on the server, so every browser sees the same. */
export interface RestartInfo {
  needed: boolean
  /** What changed ("moved to pull request #12", "updated"). */
  why?: string
  changed_unix?: number
  /** The words to show; the same everywhere. */
  notice?: string
}

export interface ResourcePage {
  id: string
  folder: string
  repository: string
  default_branch: string
  /** The release it is pinned to, when the server uses released versions. */
  pinned_commit?: string
  third_party: boolean
  /** `portal` for a paid asset imported from a zip. */
  source?: string
  version?: string
  /** Where its files are: the normal version, a branch, a pull request. */
  branch: ResourceBranch
  restart: RestartInfo
  server: { name: string; mode: 'production' | 'development'; running: boolean; console_enabled: boolean; console_waiting_for_restart: boolean }
}

export const resourceApi = {
  get: (server: string, id: string) => request<ResourcePage>(`/servers/${enc(server)}/resources/${enc(id)}`),
}

/** The wording of the restart notice (the server sends the same text; this is for the places that have only the flag). */
export const RESTART_NOTICE = 'Restart needed: the running server still has the old version.'
