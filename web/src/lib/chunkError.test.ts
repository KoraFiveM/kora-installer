import { describe, expect, it } from 'vitest'
import { isChunkLoadError } from './chunkError'

describe('isChunkLoadError', () => {
  it.each([
    ['Chrome', new TypeError('Failed to fetch dynamically imported module: http://127.0.0.1:47120/assets/SettingsPage-abc.js')],
    ['Firefox', new TypeError('error loading dynamically imported module: http://127.0.0.1:47120/assets/SettingsPage-abc.js')],
    ['Safari', new TypeError('Importing a module script failed.')],
    ['a bundler', new Error('Loading chunk 12 failed.')],
    ['a wrong file type (the shell page instead of the file)', new TypeError("Failed to load module script: Expected a JavaScript-or-Wasm module script but the server responded with a MIME type of \"text/html\".")],
  ])('knows the %s wording', (_browser, e) => expect(isChunkLoadError(e)).toBe(true))

  it('knows the error name', () => {
    const e = new Error('x')
    e.name = 'ChunkLoadError'
    expect(isChunkLoadError(e)).toBe(true)
  })

  it('does not mistake an ordinary error, or a non-error, for it', () => {
    expect(isChunkLoadError(new Error('Cannot read properties of undefined'))).toBe(false)
    expect(isChunkLoadError('Failed to fetch dynamically imported module')).toBe(false)
    expect(isChunkLoadError(null)).toBe(false)
  })
})
