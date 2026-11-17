// Types and calls of "Can players reach my server?" and "txAdmin through the panel" (kept in this file so the
// shared types.ts / client.ts need no edits). The shapes mirror crates/kora-core/src/reach and kora-web txgate.
import { request } from './client'

export type ReachState = 'ok' | 'likely' | 'problem' | 'unknown'
export type ReachFixId = 'start_server' | 'open_firewall' | 'enable_forwarding' | 'allow_cfx_list' | 'allow_ip_echo'

export interface ReachFix {
  id: ReachFixId
  label: string
}

export interface ReachLayer {
  id: 'running' | 'firewall' | 'router' | 'internet'
  state: ReachState
  title: string
  detail: string
  /** What to do by hand, one step per line, with the exact values to enter. */
  steps: string[]
  fixes: ReachFix[]
}

export interface ReachConnect {
  lan: string | null
  public: string | null
  cfx: string | null
  command: string | null
}

export interface ForwardStatus {
  at: number
  ok: boolean
  via?: string
  message: string
  external_ip?: string
  permanent: boolean
}

export interface ReachConsent {
  cfx_list: boolean
  ip_echo: boolean
}

export interface ReachReport {
  server: string
  game_port: number
  layers: ReachLayer[]
  verdict: ReachState
  headline: string
  lan_ip: string | null
  public_ip: string | null
  public_source: 'router' | 'cfx' | 'ip_echo' | null
  behind_nat: boolean | null
  cgnat: boolean
  join_code: string | null
  connect: ReachConnect
  forwarding: { enabled: boolean; status: ForwardStatus | null }
  /** What was asked outside this computer for this check, and what it learned. */
  contacted: { what: string; learned: string }[]
  /** Outside lookups that would help but are not allowed yet. */
  needs_consent: ('cfx_list' | 'ip_echo')[]
  checked_unix: number
  /** The saved choices about outside lookups. */
  consent: ReachConsent
}

export interface TxAdminTicket {
  /** Open this in a new tab: it works once, for a minute. */
  url: string
  /** The panel-side port of this server's txAdmin. */
  port: number
  expires_in: number
  /** The txAdmin account Kora created at install (never its password). */
  username: string | null
}

const enc = encodeURIComponent

export const reachApi = {
  /** `once`: allow a lookup for this check only. */
  check: (name: string, once?: Partial<ReachConsent>) => request<ReachReport>(`/servers/${enc(name)}/reach`, { method: 'POST', body: once ?? {} }),
  settings: () => request<ReachConsent>('/reach/settings'),
  setSettings: (c: Partial<ReachConsent>) => request<ReachConsent>('/reach/settings', { method: 'PUT', body: c }),
  setForwarding: (name: string, enabled: boolean) => request<{ enabled: boolean; status: ForwardStatus | null }>(`/servers/${enc(name)}/forwarding`, { method: 'PUT', body: { enabled } }),
  openFirewall: (name: string) => request<{ notes: string[] }>(`/servers/${enc(name)}/reach/firewall`, { method: 'POST', body: {} }),
  txadminTicket: (name: string) => request<TxAdminTicket>(`/servers/${enc(name)}/txadmin/ticket`, { method: 'POST', body: {} }),
}
