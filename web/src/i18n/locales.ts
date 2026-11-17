/**
 * The languages the panel can show. `en` is the source of truth (every message exists in it); the others are loaded
 * when chosen (see `load` in index.ts). `beta`: machine-assisted, not yet reviewed by a native speaker.
 * To add a language: docs/translations.md.
 */
export interface LocaleInfo {
  code: string
  /** The language's own name for itself (what the picker shows). */
  name: string
  dir: 'ltr' | 'rtl'
  beta?: boolean
  /** Only in development and in the mock build (the browser tests): never offered to people. */
  dev?: boolean
}

export const LOCALES: readonly LocaleInfo[] = [
  { code: 'en', name: 'English (UK)', dir: 'ltr' },
  { code: 'de', name: 'Deutsch', dir: 'ltr', beta: true },
  // Accented, longer English: shows what a hard-coded string or a tight layout looks like (docs/translations.md).
  { code: 'en-XA', name: 'Pseudo-language (testing)', dir: 'ltr', dev: true },
]

export const DEFAULT_LOCALE = 'en'
export const STORAGE_KEY = 'kora.locale'

/** The languages a person may pick in this build. */
export function offered(dev: boolean): LocaleInfo[] {
  return LOCALES.filter((l) => dev || !l.dev)
}

/** The best supported language for a browser's list of preferences ("de-AT", "en-US"): by the language part, else English. */
export function detect(preferred: readonly string[], dev: boolean): string {
  const have = offered(dev)
  for (const tag of preferred) {
    const exact = have.find((l) => l.code.toLowerCase() === tag.toLowerCase())
    if (exact) return exact.code
    const base = tag.split('-')[0]!.toLowerCase()
    const m = have.find((l) => l.code.toLowerCase() === base)
    if (m) return m.code
  }
  return DEFAULT_LOCALE
}

export function infoOf(code: string): LocaleInfo {
  return LOCALES.find((l) => l.code === code) ?? LOCALES[0]!
}
