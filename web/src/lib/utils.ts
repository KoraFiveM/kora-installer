import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'
import { t as tt, fmt, unix } from '@/i18n'
import '@/i18n/en/core'

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs))
}

/** 152 kB, 1.5 GB: through Intl, so the unit and the decimal mark follow the language. */
export function formatBytes(n: number): string {
  return fmt.bytes(n)
}

/** A date and time from unix seconds, in the current language. */
export function formatDate(seconds: number): string {
  return fmt.dateTime(unix(seconds))
}

/** "my server!" -> "my-server" (same rule as `kora install`). */
export function slug(name: string): string {
  let out = ''
  for (const c of name.trim()) {
    if (/[A-Za-z0-9]/.test(c)) out += c.toLowerCase()
    else if ((c === ' ' || c === '-' || c === '_') && !out.endsWith('-')) out += '-'
  }
  return out.replace(/^-+|-+$/g, '')
}

/** Ports must be whole numbers in 1024-65535 and the two must differ. Returns a message or null. */
export function validatePorts(game: string | number, txadmin: string | number): string | null {
  const g = Number(game)
  const t = Number(txadmin)
  const ok = (n: number) => Number.isInteger(n) && n >= 1024 && n <= 65535
  if (String(game).trim() === '' || String(txadmin).trim() === '' || !ok(g) || !ok(t)) return tt('core.utils.portsMustWholeNumbers')
  if (g === t) return tt('core.utils.gamePortTxadminPort')
  return null
}

export function joinPath(dir: string, name: string): string {
  const sep = dir.includes('\\') && !dir.includes('/') ? '\\' : '/'
  return dir.replace(/[\\/]+$/, '') + sep + name
}
