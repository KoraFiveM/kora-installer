import type { Mode, Source, Versions } from '@/api/types'

/**
 * The old `mode` as the two settings (production = clones of released versions, development = your own copies of the
 * latest changes). The one place the web reads that old spelling, for answers saved by an older Kora.
 */
export function styleOfMode(m: Mode): { source: Source; versions: Versions } {
  return m === 'development' ? { source: 'fork', versions: 'latest' } : { source: 'clone', versions: 'released' }
}
