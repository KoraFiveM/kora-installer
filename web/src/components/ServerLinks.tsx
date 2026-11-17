import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ExternalLink, FolderOpen } from 'lucide-react'
import { api } from '@/api/client'
import { Button } from '@/components/ui/button'
import { CopyButton } from '@/components/CopyButton'
import { toNotice } from '@/components/ErrorNotice'
import { useRole, whyNot } from '@/hooks/useRole'
import { txAdminLink, type TxAdminLink } from '@/lib/serverLinks'
import { t as tt } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/servers/serverLinks'

type Size = 'sm' | 'default'

/**
 * Is the browser on the same computer as Kora, with a desktop in front of it? Only then can a click open a
 * window the person will see. Remote (Tailscale, token), a tunnel to a headless machine and "not known yet"
 * all count as no, so they get "Copy path".
 */
export function useCanOpenFolder(): boolean {
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system })
  const { remote } = useRole()
  return !remote && sys.data?.local === true
}

/** "Open folder" on the computer that runs Kora; "Copy path" from anywhere else (see [`useCanOpenFolder`]). */
export function OpenFolderButton({ name, dir, size = 'sm' }: { name: string; dir: string; size?: Size }) {
  const canOpen = useCanOpenFolder()
  const [failed, setFailed] = useState<string | null>(null)
  if (!canOpen || failed) {
    return (
      <span className="inline-flex flex-wrap items-center gap-2">
        <CopyButton text={dir} label={tt('servers.serverLinks.copyPath')} size={size} />
        {failed && <span className="text-xs text-muted-foreground">{tt('servers.serverLinks.couldOpenHereFailed', { failed })}</span>}
      </span>
    )
  }
  return (
    <Button size={size} onClick={async () => { try { await api.servers.openFolder(name) } catch (e) { setFailed(toNotice(e)?.message ?? tt('servers.serverLinks.unknownError')) } }}>{rich('servers.serverLinks.folderopenNodeOpenFolder', { folderOpen: <FolderOpen /> })}</Button>
  )
}

/** How txAdmin is reached from this browser (see [`txAdminLink`]). */
export function useTxAdminLink(port: number): TxAdminLink {
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system })
  return txAdminLink(window.location.hostname, port, sys.data?.local === true)
}

/**
 * Opens a server's txAdmin in a new tab: straight to localhost on the computer that runs Kora, through the panel
 * everywhere else (one-time ticket, see `txgate`). Greyed out (with the reason as its tooltip) while txAdmin is not answering.
 * Opening it through the panel needs the same trust as the console (it is full control of the server), so only
 * administrators get it.
 */
export function TxAdminButton({ name, port, up, size = 'sm' }: { name: string; port: number; up: boolean; size?: Size }) {
  const link = useTxAdminLink(port)
  const role = useRole()
  const [failed, setFailed] = useState<string | null>(null)
  const [note, setNote] = useState<{ username: string | null; url: string | null } | null>(null)
  if (!up) return <Button size={size} variant="default" disabled title={tt('servers.serverLinks.txadminAnswersOnceServer')}>{rich('servers.serverLinks.externallinkNodeOpenTxadmin', { externalLink: <ExternalLink /> })}</Button>
  if (!link.viaPanel && link.url) {
    return (
      <Button asChild size={size} variant="default">
        <a href={link.url} target="_blank" rel="noopener noreferrer" title={link.hint ?? undefined}>{rich('servers.serverLinks.externallinkNodeOpenTxadmin', { externalLink: <ExternalLink /> })}</a>
      </Button>
    )
  }
  // txAdmin can run code on the server: nobody below administrator sees the button at all.
  if (!role.can('admin')) return null
  const off = whyNot(role, 'admin', true)
  const open = async () => {
    setFailed(null)
    // Opened first, in the click, so the browser does not take it for a pop-up; pointed at txAdmin once the ticket is there.
    const w = window.open('about:blank', '_blank')
    try {
      const t = await api.reach.txadminTicket(name)
      if (w) {
        w.opener = null
        w.location.href = t.url
        setNote({ username: t.username, url: null })
      } else {
        setNote({ username: t.username, url: t.url })
      }
    } catch (e) {
      w?.close()
      setFailed(toNotice(e)?.message ?? tt('servers.serverLinks.unknownError'))
    }
  }
  return (
    <span className="inline-flex flex-col items-start gap-1">
      <Button size={size} variant="default" disabled={off !== null} title={off ?? link.hint ?? undefined} onClick={() => void open()}>{rich('servers.serverLinks.externallinkNodeOpenTxadmin', { externalLink: <ExternalLink /> })}</Button>
      {off && <span className="max-w-sm text-xs text-muted-foreground">{off}</span>}
      {failed && <span className="max-w-sm text-xs text-destructive">{failed}</span>}
      {note?.url && <a className="text-xs underline" href={note.url} target="_blank" rel="noopener noreferrer">{tt('servers.serverLinks.browserBlockedNewTab')}</a>}
      {note && <span className="max-w-sm text-xs text-muted-foreground">{rich('servers.serverLinks.txadminItsOwnSignin', { node: note.username ? <>{rich('servers.serverLinks.useAccountStrongNode', { strong: <strong>{note.username}</strong>, code: (c) => <code className="font-mono">{c}</code> })}</> : '.' })}</span>}
    </span>
  )
}

/** Copies the address players on the network type to join. Nothing when Kora could not work it out. */
export function CopyConnectButton({ connect, size = 'sm' }: { connect: string | null | undefined; size?: Size }) {
  if (!connect) return null
  return <CopyButton text={connect} label={tt('servers.serverLinks.copyConnectAddress')} size={size} />
}
