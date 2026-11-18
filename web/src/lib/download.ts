import { sessionSecret } from '@/api/client'
import { t } from '@/i18n'
import '@/i18n/en/core/download'

/** Largest file saved through memory when the browser cannot save a stream straight to disk. */
export const BLOB_LIMIT = 1024 * 1024 * 1024

interface SaveFilePicker {
  showSaveFilePicker?: (o: { suggestedName: string }) => Promise<{ createWritable: () => Promise<{ write: (b: Uint8Array) => Promise<void>; close: () => Promise<void> }> }>
}

/**
 * Download a file from the panel with the session header (a plain link would not carry it, and the secret must
 * not go in an address). Where the browser can save a stream to a file (Chromium) the file goes straight to
 * disk; elsewhere it is held in memory, which is refused above [`BLOB_LIMIT`].
 */
export async function downloadFile(url: string, fileName: string, size: number): Promise<void> {
  const picker = (window as unknown as SaveFilePicker).showSaveFilePicker
  if (!picker && size > BLOB_LIMIT) throw new Error(t('core.download.fileTooBigBrowser'))
  const secret = sessionSecret()
  const res = await fetch(url, { headers: secret ? { 'X-Kora-Session': secret } : {} })
  if (!res.ok || !res.body) throw new Error(res.status === 403 ? t('core.download.accountMayDownloadBackups') : t('core.download.downloadFailedStatus', { status: res.status }))
  if (picker) {
    const handle = await picker.call(window, { suggestedName: fileName })
    const out = await handle.createWritable()
    const reader = res.body.getReader()
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      await out.write(value)
    }
    await out.close()
    return
  }
  const blob = await res.blob()
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = fileName
  a.click()
  setTimeout(() => URL.revokeObjectURL(a.href), 10_000)
}
