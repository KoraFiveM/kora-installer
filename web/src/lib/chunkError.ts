/**
 * Is this the error of a code-split file that could not be loaded?
 *
 * The panel binary embeds the UI. After Kora updates itself, a tab that was opened before the update still
 * runs the OLD main file, which asks for chunk files (`/assets/Settings-3fa9c1.js`) that the new binary no
 * longer has: the server answers 404 for them (see `static_ui.rs`), and the import fails. A lost connection
 * fails the same way. Both need the same thing from the person: reload the page.
 *
 * The browsers word it differently: Chrome and Edge "Failed to fetch dynamically imported module", Firefox
 * "error loading dynamically imported module", Safari "Importing a module script failed", and bundlers'
 * own "Loading chunk 12 failed" / `ChunkLoadError`.
 */
export function isChunkLoadError(e: unknown): boolean {
  if (!(e instanceof Error)) return false
  if (e.name === 'ChunkLoadError') return true
  return /dynamically imported module|importing a module script failed|loading (css )?chunk [\w-]+ failed|failed to load module script|unable to preload css/i.test(e.message)
}
