import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type * as T from '@/api/types'
import type { RemoteSettingsUpdate, ResetWhat, Role } from '@/api/types'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Input, Select } from '@/components/ui/input'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'

const ROLE_TEXT: Record<Role, string> = {
  viewer: 'Viewer: can look at servers and status, nothing else',
  operator: 'Operator: can also start, stop and restart, update, and read logs and the console',
  admin: 'Admin: everything (deleting and similar needs the switch below)',
}

function RoleSelect({ value, onChange, label }: { value: Role; onChange: (r: Role) => void; label: string }) {
  return (
    <Select aria-label={label} value={value} onChange={(e) => onChange(e.target.value as Role)}>
      <option value="viewer">Viewer</option>
      <option value="operator">Operator</option>
      <option value="admin">Admin</option>
    </Select>
  )
}

/** The address a new person opens to set up: the Tailscale or allowed name when there is one, else this page's address. */
export function enrolUrl(s: { tailnet_url: string | null; allowed_hosts: string[] } | undefined, link: string, secure = window.location.protocol === 'https:'): string {
  const host = s?.allowed_hosts[0]
  const base = s?.tailnet_url ?? (host ? `${secure ? 'https' : 'http'}://${host}${host.includes(':') && !host.startsWith('[') ? '' : `:${window.location.port || (secure ? 443 : 80)}`}` : window.location.origin)
  return `${base}/#enrol=${link}`
}

function howSignsIn(u: { passkeys: number; code: boolean; legacy_token: boolean }): string {
  const how: string[] = []
  if (u.passkeys > 0) how.push(`${u.passkeys} passkey${u.passkeys === 1 ? '' : 's'}`)
  if (u.code) how.push('one-time codes')
  if (u.legacy_token) how.push('old token link')
  return how.length ? how.join(', ') : 'waiting to set up'
}

function Users({ settings }: { settings: T.RemoteSettings | undefined }) {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['panel-users'], queryFn: api.users.list })
  const [name, setName] = useState('')
  const [role, setRole] = useState<Role>('operator')
  const [fresh, setFresh] = useState<{ name: string; link: string; what: string } | null>(null)
  const [removing, setRemoving] = useState<string | null>(null)
  const [resetting, setResetting] = useState<string | null>(null)
  const [what, setWhat] = useState<ResetWhat>('all')
  const refresh = () => qc.invalidateQueries({ queryKey: ['panel-users'] })
  const add = useMutation({
    mutationFn: () => api.users.add(name.trim(), role),
    onSuccess: (u) => { setFresh({ name: u.name, link: u.link, what: 'invited' }); setName(''); void refresh() },
  })
  const reset = useMutation({
    mutationFn: (n: string) => api.users.reset(n, what),
    onSuccess: (r) => { setFresh(r.link ? { name: r.name, link: r.link, what: 'reset' } : null); setResetting(null); void refresh() },
  })
  const remove = useMutation({ mutationFn: (n: string) => api.users.remove(n), onSuccess: () => void refresh(), onSettled: () => setRemoving(null) })
  return (
    <div className="space-y-3">
      <h3 className="text-sm font-semibold">People</h3>
      {(q.data ?? []).length === 0 && <p className="text-sm text-muted-foreground">Nobody yet.</p>}
      <ul className="divide-y divide-border rounded-md border border-border">
        {(q.data ?? []).map((u) => (
          <li key={u.name} className="flex flex-wrap items-center justify-between gap-3 px-3 py-2 text-sm">
            <span>
              <strong>{u.name}</strong> <Badge tone="plain">{u.role}</Badge>
              <span className="block text-xs text-muted-foreground">Signs in with: {howSignsIn(u)}</span>
            </span>
            <span className="flex gap-2">
              <Button size="sm" variant="secondary" onClick={() => { setWhat('all'); setResetting(u.name) }} aria-label={`Reset sign-in for ${u.name}`}>Reset sign-in</Button>
              <Button size="sm" variant="destructive" disabled={remove.isPending} onClick={() => setRemoving(u.name)} aria-label={`Remove ${u.name}`}>Remove</Button>
            </span>
          </li>
        ))}
      </ul>
      <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => { e.preventDefault(); if (name.trim()) add.mutate() }}>
        <div className="w-44"><Field label="Name"><Input value={name} onChange={(e) => setName(e.target.value)} autoComplete="off" /></Field></div>
        <div className="w-40"><Field label="Role"><RoleSelect label="Role" value={role} onChange={setRole} /></Field></div>
        <Button type="submit" disabled={!name.trim() || add.isPending}>Invite person</Button>
      </form>
      <p className="text-xs text-muted-foreground">{ROLE_TEXT[role]}</p>
      {fresh && (
        <Alert tone="success" title={fresh.what === 'invited' ? `Invitation for ${fresh.name}` : `New set-up link for ${fresh.name}`}>
          <p className="mb-2">Send this link to them privately. It works once, for 30 minutes, and they set up a passkey or one-time codes on it. It cannot be shown again.</p>
          <CopyField value={enrolUrl(settings, fresh.link)} label={`Enrolment link for ${fresh.name}`} />
        </Alert>
      )}
      <ErrorNotice error={add.error ?? remove.error ?? reset.error ?? q.error} />
      <ConfirmDialog
        open={removing !== null}
        onOpenChange={(o) => { if (!o) setRemoving(null) }}
        title={`Remove ${removing ?? ''}?`}
        description="They are signed out, their passkeys and codes are deleted, and they cannot sign in again until you invite them back."
        confirmLabel="Remove"
        destructive
        busy={remove.isPending}
        onConfirm={() => { if (removing) remove.mutate(removing) }}
      />
      <ConfirmDialog
        open={resetting !== null}
        onOpenChange={(o) => { if (!o) setResetting(null) }}
        title={`Reset the sign-in of ${resetting ?? ''}?`}
        description="Every device they are signed in on is signed out at once, and you get a new one-time link for them to set up again."
        confirmLabel="Reset and make a link"
        destructive
        busy={reset.isPending}
        onConfirm={() => { if (resetting) reset.mutate(resetting) }}
      >
        <Field label="What to take away">
          <Select aria-label="What to take away" value={what} onChange={(e) => setWhat(e.target.value as ResetWhat)}>
            <option value="all">Everything (passkeys, codes, old token link)</option>
            <option value="passkeys">Only their passkeys</option>
            <option value="code">Only their password and one-time codes</option>
            <option value="legacy_token">Only the old token link (no new link)</option>
          </Select>
        </Field>
      </ConfirmDialog>
    </div>
  )
}

function ApiTokens() {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['api-tokens'], queryFn: api.apiTokens.list })
  const [name, setName] = useState('')
  const [role, setRole] = useState<Role>('operator')
  const [days, setDays] = useState('')
  const [fresh, setFresh] = useState<{ name: string; token: string } | null>(null)
  const [revoking, setRevoking] = useState<{ id: number; name: string } | null>(null)
  const refresh = () => qc.invalidateQueries({ queryKey: ['api-tokens'] })
  const create = useMutation({
    mutationFn: () => api.apiTokens.create({ name: name.trim(), role, ...(days.trim() ? { days: Number(days) } : {}) }),
    onSuccess: (t) => { setFresh({ name: t.name, token: t.token }); setName(''); setDays(''); void refresh() },
  })
  const revoke = useMutation({ mutationFn: (id: number) => api.apiTokens.revoke(id), onSuccess: () => void refresh(), onSettled: () => setRevoking(null) })
  const day = (unix: number) => (unix ? new Date(unix * 1000).toLocaleDateString('en-GB') : 'never')
  return (
    <div className="space-y-3">
      <h3 className="text-sm font-semibold">API tokens for scripts</h3>
      <p className="text-xs text-muted-foreground">For a script or a backup job, not for people. Sent as an <code className="font-mono">Authorization: Bearer</code> header only: never in an address, and never a way to sign in to the page. Deleting and uninstalling through a token still needs the switch below.</p>
      {(q.data ?? []).length > 0 && (
        <ul className="divide-y divide-border rounded-md border border-border">
          {(q.data ?? []).map((t) => (
            <li key={t.id} className="flex flex-wrap items-center justify-between gap-3 px-3 py-2 text-sm">
              <span>
                <strong>{t.name}</strong> <Badge tone="plain">{t.role}</Badge>
                <span className="block text-xs text-muted-foreground">Expires {day(t.expires)}, last used {t.last_used ? day(t.last_used) : 'never'}</span>
              </span>
              <Button size="sm" variant="destructive" onClick={() => setRevoking({ id: t.id, name: t.name })} aria-label={`Revoke ${t.name}`}>Revoke</Button>
            </li>
          ))}
        </ul>
      )}
      <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => { e.preventDefault(); if (name.trim()) create.mutate() }}>
        <div className="w-44"><Field label="Token name" hint="For example backup-script"><Input value={name} onChange={(e) => setName(e.target.value)} autoComplete="off" /></Field></div>
        <div className="w-36"><Field label="Role"><RoleSelect label="Role for this token" value={role} onChange={setRole} /></Field></div>
        <div className="w-32"><Field label="Days valid" hint="Empty: no expiry"><Input inputMode="numeric" value={days} onChange={(e) => setDays(e.target.value.replace(/\D/g, ''))} /></Field></div>
        <Button type="submit" disabled={!name.trim() || create.isPending}>Make token</Button>
      </form>
      {fresh && (
        <Alert tone="success" title={`Token for ${fresh.name}`}>
          <p className="mb-2">Copy it now. It is not stored and cannot be shown again.</p>
          <CopyField value={fresh.token} label={`API token for ${fresh.name}`} />
        </Alert>
      )}
      <ErrorNotice error={create.error ?? revoke.error ?? q.error} />
      <ConfirmDialog
        open={revoking !== null}
        onOpenChange={(o) => { if (!o) setRevoking(null) }}
        title={`Revoke ${revoking?.name ?? ''}?`}
        description="Anything using this token stops working at once."
        confirmLabel="Revoke"
        destructive
        busy={revoke.isPending}
        onConfirm={() => { if (revoking) revoke.mutate(revoking.id) }}
      />
    </div>
  )
}

/** Settings > Remote access. Only shown on this computer: remote sessions cannot change who gets in. */
export function RemoteAccessCard() {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['remote'], queryFn: api.remote.get })
  const save = useMutation({ mutationFn: (b: RemoteSettingsUpdate) => api.remote.set(b), onSuccess: (d) => qc.setQueryData(['remote'], d) })
  const [host, setHost] = useState('')
  const [login, setLogin] = useState('')
  const [loginRole, setLoginRole] = useState<Role>('viewer')
  const [removing, setRemoving] = useState<string | null>(null)
  const s = q.data
  const loopback = s ? s.bind === '127.0.0.1' || s.bind === '::1' : true
  return (
    <Card>
      <CardHeader><CardTitle>Remote access</CardTitle></CardHeader>
      <CardContent className="space-y-5">
        <p className="text-sm text-muted-foreground">By default only this computer can open the panel. Remote access lets other people open it from their own computers. Over a private network such as Tailscale the connection is already encrypted; on a home network or the internet the panel needs HTTPS (see HTTPS below).</p>
        {s && (
          <>
            <Checkbox label="Allow remote access" checked={s.enabled} disabled={save.isPending} onChange={(e) => save.mutate({ enabled: e.target.checked })}
              hint={s.enabled_by_flag ? 'It is on for this run only because the panel was started with --remote.' : undefined} />
            <div className="text-sm">
              Listening on <code className="font-mono text-xs">{s.bind}:{s.port}</code>{' '}
              {s.tailnet_url ? <Badge tone="plain">this computer and your Tailscale network</Badge> : loopback ? <Badge tone="plain">this computer only</Badge> : <Badge tone="amber">reachable from the network</Badge>}
            </div>
            {s.tailnet_url && <Alert tone="info">Your Tailscale devices open the panel at <code className="font-mono">{s.tailnet_url}</code> and sign in with their Tailscale account. Only the accounts listed below get in. Turn it off with <code className="font-mono">kora panel tailscale off</code>.</Alert>}
            {loopback && !s.tailnet_url && <Alert tone="info">To open the panel on your own phone or laptop, restart it with <code className="font-mono">kora panel --tailscale</code>. It then listens on your Tailscale address only, and you are signed in with your Tailscale account, with no token to copy. Do not put the panel behind a proxy or <code className="font-mono">tailscale serve</code>: it refuses those requests.</Alert>}

            <div className="space-y-3">
              <Checkbox label="Let people sign in with Tailscale" checked={s.tailscale} disabled={save.isPending} onChange={(e) => save.mutate({ tailscale: e.target.checked })}
                hint="Kora asks Tailscale who is connecting. Only the accounts listed here get in." />
              {s.tailscale && (
                <div className="space-y-2">
                  <ul className="divide-y divide-border rounded-md border border-border">
                    {Object.entries(s.tailscale_users).map(([l, r]) => (
                      <li key={l} className="flex items-center justify-between gap-3 px-3 py-2 text-sm">
                        <span><strong>{l}</strong> <Badge tone="plain">{r}</Badge></span>
                        <Button size="sm" variant="destructive" onClick={() => setRemoving(l)} aria-label={`Remove ${l}`}>Remove</Button>
                      </li>
                    ))}
                  </ul>
                  <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => { e.preventDefault(); const l = login.trim().toLowerCase(); if (!l) return; save.mutate({ tailscale_users: { ...s.tailscale_users, [l]: loginRole } }); setLogin('') }}>
                    <div className="w-56"><Field label="Tailscale account" hint="For example name@example.com"><Input value={login} onChange={(e) => setLogin(e.target.value)} autoComplete="off" /></Field></div>
                    <div className="w-40"><Field label="Role"><RoleSelect label="Role for this account" value={loginRole} onChange={setLoginRole} /></Field></div>
                    <Button type="submit" disabled={!login.trim() || save.isPending}>Allow account</Button>
                  </form>
                </div>
              )}
            </div>

            <Users settings={s} />
            <ApiTokens />

            <div className="space-y-2">
              <h3 className="text-sm font-semibold">Names people will type</h3>
              <p className="text-xs text-muted-foreground">The panel's own address and your Tailscale name are always accepted. Add any other name here, such as a DNS name.</p>
              <ul className="space-y-1">
                {s.allowed_hosts.map((h) => (
                  <li key={h} className="flex items-center justify-between gap-3 text-sm"><code className="font-mono text-xs">{h}</code>
                    <Button size="sm" variant="secondary" onClick={() => save.mutate({ allowed_hosts: s.allowed_hosts.filter((x) => x !== h) })} aria-label={`Remove ${h}`}>Remove</Button></li>
                ))}
              </ul>
              <form className="flex items-end gap-3" onSubmit={(e) => { e.preventDefault(); const h = host.trim(); if (!h) return; save.mutate({ allowed_hosts: [...s.allowed_hosts, h] }); setHost('') }}>
                <div className="w-64"><Field label="Add a name"><Input value={host} onChange={(e) => setHost(e.target.value)} autoComplete="off" /></Field></div>
                <Button type="submit" disabled={!host.trim() || save.isPending}>Add</Button>
              </form>
            </div>

            <div className="space-y-2">
              <Checkbox label="Require a passkey or one-time codes for everybody" checked={s.require_strong_auth} disabled={save.isPending} onChange={(e) => save.mutate({ require_strong_auth: e.target.checked })}
                hint="People can no longer sign in to the page with an old token link. Scripts that send a token as a header are not affected. Turn it on once everybody has set up a passkey or one-time codes." />
            </div>

            <div className="space-y-2">
              <Checkbox label="Let remote administrators do destructive things" checked={s.allow_remote_destructive} disabled={save.isPending} onChange={(e) => save.mutate({ allow_remote_destructive: e.target.checked })}
                hint="Deleting or removing servers, uninstalling Kora, sending console commands, changing schedules and updating Kora, for administrators who do not sign in through Tailscale (passkey, one-time codes, tokens). Off by default. Administrators signed in through Tailscale can always do these, and every remote administrator can install servers. Even when on, deleting still asks for the server's name, and a passkey or code check is asked for first." />
              {s.allow_remote_destructive && <Alert tone="warning">Remote administrators (and anyone who holds an admin token) can now delete servers from afar. Only turn this on if you trust every admin and keep their sign-ins safe.</Alert>}
            </div>
            {s.saved_to && <p className="text-xs text-muted-foreground">Saved in {s.saved_to}</p>}
          </>
        )}
        <ErrorNotice error={save.error ?? q.error} />
        <ConfirmDialog
          open={removing !== null}
          onOpenChange={(o) => { if (!o) setRemoving(null) }}
          title={`Remove ${removing ?? ''}?`}
          description="They are signed out and cannot sign in again until you add them back."
          confirmLabel="Remove"
          destructive
          busy={save.isPending}
          onConfirm={() => {
            if (!s || !removing) return
            const next = { ...s.tailscale_users }
            delete next[removing]
            save.mutate({ tailscale_users: next }, { onSettled: () => setRemoving(null) })
          }}
        />
      </CardContent>
    </Card>
  )
}

/** For someone signed in from another computer: who they are and what they can do. */
export function SignedInCard() {
  const r = useRole()
  const out = useMutation({ mutationFn: api.session.logout, onSuccess: () => window.location.reload() })
  if (!r.identity) return null
  return (
    <Card>
      <CardHeader><CardTitle>Your access</CardTitle></CardHeader>
      <CardContent className="space-y-3 text-sm">
        <p>Signed in as <strong>{r.identity.name}</strong> (<Badge tone="plain">{r.role}</Badge>){r.remote ? ', from another computer' : ''}.</p>
        <p className="text-muted-foreground">{ROLE_TEXT[r.role]}.</p>
        {r.remote && !r.destructiveAllowed && <p className="text-muted-foreground">Deleting servers, uninstalling, console commands, schedules and updating Kora are turned off for sign-ins that do not come through Tailscale. Installing and managing servers still work.</p>}
        <Button onClick={() => out.mutate()} disabled={out.isPending}>Sign out</Button>
        <ErrorNotice error={out.error} />
      </CardContent>
    </Card>
  )
}
