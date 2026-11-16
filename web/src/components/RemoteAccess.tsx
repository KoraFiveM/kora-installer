import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { RemoteSettingsUpdate, Role } from '@/api/types'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Input, Select } from '@/components/ui/input'
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

function Users() {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['panel-users'], queryFn: api.users.list })
  const [name, setName] = useState('')
  const [role, setRole] = useState<Role>('operator')
  const [fresh, setFresh] = useState<{ name: string; token: string } | null>(null)
  const refresh = () => qc.invalidateQueries({ queryKey: ['panel-users'] })
  const add = useMutation({
    mutationFn: () => api.users.add(name.trim(), role),
    onSuccess: (u) => { setFresh({ name: u.name, token: u.token }); setName(''); void refresh() },
  })
  const remove = useMutation({ mutationFn: (n: string) => api.users.remove(n), onSuccess: () => void refresh() })
  return (
    <div className="space-y-3">
      <h3 className="text-sm font-semibold">People with a token</h3>
      {(q.data ?? []).length === 0 && <p className="text-sm text-muted-foreground">Nobody yet.</p>}
      <ul className="divide-y divide-border rounded-md border border-border">
        {(q.data ?? []).map((u) => (
          <li key={u.name} className="flex items-center justify-between gap-3 px-3 py-2 text-sm">
            <span><strong>{u.name}</strong> <Badge tone="plain">{u.role}</Badge></span>
            <Button size="sm" variant="destructive" disabled={remove.isPending} onClick={() => remove.mutate(u.name)} aria-label={`Remove ${u.name}`}>Remove</Button>
          </li>
        ))}
      </ul>
      <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => { e.preventDefault(); if (name.trim()) add.mutate() }}>
        <div className="w-44"><Field label="Name"><Input value={name} onChange={(e) => setName(e.target.value)} autoComplete="off" /></Field></div>
        <div className="w-40"><Field label="Role"><RoleSelect label="Role" value={role} onChange={setRole} /></Field></div>
        <Button type="submit" disabled={!name.trim() || add.isPending}>Add person</Button>
      </form>
      <p className="text-xs text-muted-foreground">{ROLE_TEXT[role]}</p>
      {fresh && (
        <Alert tone="success" title={`Token for ${fresh.name}`}>
          <p className="mb-2">Copy it now and send it to them privately. It is not stored and cannot be shown again.</p>
          <CopyField value={fresh.token} label={`Token for ${fresh.name}`} />
        </Alert>
      )}
      <ErrorNotice error={add.error ?? remove.error ?? q.error} />
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
  const s = q.data
  const loopback = s ? s.bind === '127.0.0.1' || s.bind === '::1' : true
  return (
    <Card>
      <CardHeader><CardTitle>Remote access</CardTitle></CardHeader>
      <CardContent className="space-y-5">
        <p className="text-sm text-muted-foreground">By default only this computer can open the panel. Remote access lets other people open it from their own computers. The connection is plain HTTP, not encrypted, so use it only over a private network such as Tailscale or WireGuard, or through an SSH tunnel.</p>
        {s && (
          <>
            <Checkbox label="Allow remote access" checked={s.enabled} disabled={save.isPending} onChange={(e) => save.mutate({ enabled: e.target.checked })}
              hint={s.enabled_by_flag ? 'It is on for this run only because the panel was started with --remote.' : undefined} />
            <div className="text-sm">
              Listening on <code className="font-mono text-xs">{s.bind}:{s.port}</code>{' '}
              {loopback ? <Badge tone="plain">this computer only</Badge> : <Badge tone="amber">reachable from the network</Badge>}
            </div>
            {loopback && <Alert tone="info">To open the panel on your own phone or laptop, restart it with <code className="font-mono">kora panel --tailscale</code>. It then listens on your Tailscale address only, and you are signed in with your Tailscale account, with no token to copy. Do not put the panel behind a proxy or <code className="font-mono">tailscale serve</code>: it refuses those requests.</Alert>}

            <div className="space-y-3">
              <Checkbox label="Let people sign in with Tailscale" checked={s.tailscale} disabled={save.isPending} onChange={(e) => save.mutate({ tailscale: e.target.checked })}
                hint="Kora asks Tailscale who is connecting. Only the accounts listed here get in." />
              {s.tailscale && (
                <div className="space-y-2">
                  <ul className="divide-y divide-border rounded-md border border-border">
                    {Object.entries(s.tailscale_users).map(([l, r]) => (
                      <li key={l} className="flex items-center justify-between gap-3 px-3 py-2 text-sm">
                        <span><strong>{l}</strong> <Badge tone="plain">{r}</Badge></span>
                        <Button size="sm" variant="destructive" onClick={() => { const next = { ...s.tailscale_users }; delete next[l]; save.mutate({ tailscale_users: next }) }} aria-label={`Remove ${l}`}>Remove</Button>
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

            <Users />

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
              <Checkbox label="Let remote administrators do destructive things" checked={s.allow_remote_destructive} disabled={save.isPending} onChange={(e) => save.mutate({ allow_remote_destructive: e.target.checked })}
                hint="Deleting or removing servers, uninstalling, sending console commands, installing, changing schedules and updating Kora. Off by default. Even when on, deleting still asks for the server's name." />
              {s.allow_remote_destructive && <Alert tone="warning">Anyone with an admin role can now delete servers from afar. Only turn this on if you trust every admin.</Alert>}
            </div>
            {s.saved_to && <p className="text-xs text-muted-foreground">Saved in {s.saved_to}</p>}
          </>
        )}
        <ErrorNotice error={save.error ?? q.error} />
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
        {r.remote && !r.destructiveAllowed && <p className="text-muted-foreground">Deleting, uninstalling and similar actions are turned off for remote access.</p>}
        <Button onClick={() => out.mutate()} disabled={out.isPending}>Sign out</Button>
        <ErrorNotice error={out.error} />
      </CardContent>
    </Card>
  )
}
