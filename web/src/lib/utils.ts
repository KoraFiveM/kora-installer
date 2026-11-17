import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs))
}

export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`
  if (n >= 1e3) return `${Math.round(n / 1e3)} KB`
  return `${n} B`
}

export function formatDate(unix: number): string {
  return new Date(unix * 1000).toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' })
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
  if (String(game).trim() === '' || String(txadmin).trim() === '' || !ok(g) || !ok(t)) return 'Ports must be whole numbers from 1024 to 65535.'
  if (g === t) return 'The game port and the txAdmin port must be different.'
  return null
}

export function joinPath(dir: string, name: string): string {
  const sep = dir.includes('\\') && !dir.includes('/') ? '\\' : '/'
  return dir.replace(/[\\/]+$/, '') + sep + name
}
