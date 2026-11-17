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
import { t as tt, msg, type Msg } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/settings'

const ROLE_TEXT: Record<Role, Msg> = {
  viewer: msg('settings.remoteAccess.viewerLookServersStatus'),
  operator: msg('settings.remoteAccess.operatorAlsoStartStop'),
  admin: msg('settings.remoteAccess.adminEverythingDeletingSimilar'),
}

function RoleSelect({ value, onChange, label }: { value: Role; onChange: (r: Role) => void; label: string }) {
  return (
    <Select aria-label={label} value={value} onChange={(e) => onChange(e.target.value as Role)}>
      <option value="viewer">{tt('settings.remoteAccess.viewer')}</option>
      <option value="operator">{tt('settings.remoteAccess.operator')}</option>
      <option value="admin">{tt('settings.remoteAccess.admin')}</option>
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
      <h3 className="text-sm font-semibold">{tt('settings.remoteAccess.people')}</h3>
      {(q.data ?? []).length === 0 && <p className="text-sm text-muted-foreground">{tt('settings.remoteAccess.nobodyYet')}</p>}
      <ul className="divide-y divide-border rounded-md border border-border">
        {(q.data ?? []).map((u) => (
          <li key={u.name} className="flex flex-wrap items-center justify-between gap-3 px-3 py-2 text-sm">
            <span>
              <strong>{u.name}</strong> <Badge tone="plain">{u.role}</Badge>
              <span className="block text-xs text-muted-foreground">{tt('settings.remoteAccess.signsHowsignsin', { howSignsIn: howSignsIn(u) })}</span>
            </span>
            <span className="flex gap-2">
              <Button size="sm" variant="secondary" onClick={() => { setWhat('all'); setResetting(u.name) }} aria-label={tt('settings.remoteAccess.resetSigninName', { name: u.name })}>{tt('settings.remoteAccess.resetSignin')}</Button>
              <Button size="sm" variant="destructive" disabled={remove.isPending} onClick={() => setRemoving(u.name)} aria-label={tt('settings.remoteAccess.removeName', { name: u.name })}>{tt('settings.remoteAccess.remove')}</Button>
            </span>
          </li>
        ))}
      </ul>
      <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => { e.preventDefault(); if (name.trim()) add.mutate() }}>
        <div className="w-44"><Field label={tt('settings.remoteAccess.name')}><Input value={name} onChange={(e) => setName(e.target.value)} autoComplete="off" /></Field></div>
        <div className="w-40"><Field label={tt('settings.remoteAccess.role')}><RoleSelect label={tt('settings.remoteAccess.role')} value={role} onChange={setRole} /></Field></div>
        <Button type="submit" disabled={!name.trim() || add.isPending}>{tt('settings.remoteAccess.invitePerson')}</Button>
      </form>
      <p className="text-xs text-muted-foreground">{tt(ROLE_TEXT[role])}</p>
      {fresh && (
        <Alert tone="success" title={fresh.what === 'invited' ? tt('settings.remoteAccess.invitationName', { name: fresh.name }) : tt('settings.remoteAccess.newSetupLinkName', { name: fresh.name })}>
          <p className="mb-2">{tt('settings.remoteAccess.sendLinkThemPrivately')}</p>
          <CopyField value={enrolUrl(settings, fresh.link)} label={tt('settings.remoteAccess.enrolmentLinkName', { name: fresh.name })} />
        </Alert>
      )}
      <ErrorNotice error={add.error ?? remove.error ?? reset.error ?? q.error} />
      <ConfirmDialog
        open={removing !== null}
        onOpenChange={(o) => { if (!o) setRemoving(null) }}
        title={tt('settings.remoteAccess.removeValue', { value: removing ?? '' })}
        description={tt('settings.remoteAccess.theySignedOutTheir')}
        confirmLabel={tt('settings.remoteAccess.remove')}
        destructive
        busy={remove.isPending}
        onConfirm={() => { if (removing) remove.mutate(removing) }}
      />
      <ConfirmDialog
        open={resetting !== null}
        onOpenChange={(o) => { if (!o) setResetting(null) }}
        title={tt('settings.remoteAccess.resetSigninValue', { value: resetting ?? '' })}
        description={tt('settings.remoteAccess.everyDeviceTheySigned')}
        confirmLabel={tt('settings.remoteAccess.resetMakeLink')}
        destructive
        busy={reset.isPending}
        onConfirm={() => { if (resetting) reset.mutate(resetting) }}
      >
        <Field label={tt('settings.remoteAccess.whatTakeAway')}>
          <Select aria-label={tt('settings.remoteAccess.whatTakeAway')} value={what} onChange={(e) => setWhat(e.target.value as ResetWhat)}>
            <option value="all">{tt('settings.remoteAccess.everythingPasskeysCodesOld')}</option>
            <option value="passkeys">{tt('settings.remoteAccess.onlyTheirPasskeys')}</option>
            <option value="code">{tt('settings.remoteAccess.onlyTheirPasswordOnetime')}</option>
            <option value="legacy_token">{tt('settings.remoteAccess.onlyOldTokenLink')}</option>
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
      <h3 className="text-sm font-semibold">{tt('settings.remoteAccess.apiTokensScripts')}</h3>
      <p className="text-xs text-muted-foreground">{rich('settings.remoteAccess.scriptBackupJobPeople', { code: (c) => <code className="font-mono">{c}</code> })}</p>
      {(q.data ?? []).length > 0 && (
        <ul className="divide-y divide-border rounded-md border border-border">
          {(q.data ?? []).map((t) => (
            <li key={t.id} className="flex flex-wrap items-center justify-between gap-3 px-3 py-2 text-sm">
              <span>
                <strong>{t.name}</strong> <Badge tone="plain">{t.role}</Badge>
                <span className="block text-xs text-muted-foreground">{tt('settings.remoteAccess.expiresDayLastUsed', { day: day(t.expires), value: t.last_used ? day(t.last_used) : 'never' })}</span>
              </span>
              <Button size="sm" variant="destructive" onClick={() => setRevoking({ id: t.id, name: t.name })} aria-label={tt('settings.remoteAccess.revokeName', { name: t.name })}>{tt('settings.remoteAccess.revoke')}</Button>
            </li>
          ))}
        </ul>
      )}
      <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => { e.preventDefault(); if (name.trim()) create.mutate() }}>
        <div className="w-44"><Field label={tt('settings.remoteAccess.tokenName')} hint={tt('settings.remoteAccess.exampleBackupscript')}><Input value={name} onChange={(e) => setName(e.target.value)} autoComplete="off" /></Field></div>
        <div className="w-36"><Field label={tt('settings.remoteAccess.role')}><RoleSelect label={tt('settings.remoteAccess.roleToken')} value={role} onChange={setRole} /></Field></div>
        <div className="w-32"><Field label={tt('settings.remoteAccess.daysValid')} hint={tt('settings.remoteAccess.emptyNoExpiry')}><Input inputMode="numeric" value={days} onChange={(e) => setDays(e.target.value.replace(/\D/g, ''))} /></Field></div>
        <Button type="submit" disabled={!name.trim() || create.isPending}>{tt('settings.remoteAccess.makeToken')}</Button>
      </form>
      {fresh && (
        <Alert tone="success" title={tt('settings.remoteAccess.tokenName2', { name: fresh.name })}>
          <p className="mb-2">{tt('settings.remoteAccess.copyNowStoredCannot')}</p>
          <CopyField value={fresh.token} label={tt('settings.remoteAccess.apiTokenName', { name: fresh.name })} />
        </Alert>
      )}
      <ErrorNotice error={create.error ?? revoke.error ?? q.error} />
      <ConfirmDialog
        open={revoking !== null}
        onOpenChange={(o) => { if (!o) setRevoking(null) }}
        title={tt('settings.remoteAccess.revokeValue', { value: revoking?.name ?? '' })}
        description={tt('settings.remoteAccess.anythingUsingTokenStops')}
        confirmLabel={tt('settings.remoteAccess.revoke')}
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
      <CardHeader><CardTitle>{tt('settings.remoteAccess.remoteAccess')}</CardTitle></CardHeader>
      <CardContent className="space-y-5">
        <p className="text-sm text-muted-foreground">{tt('settings.remoteAccess.defaultOnlyComputerOpen')}</p>
        {s && (
          <>
            <Checkbox label={tt('settings.remoteAccess.allowRemoteAccess')} checked={s.enabled} disabled={save.isPending} onChange={(e) => save.mutate({ enabled: e.target.checked })}
              hint={s.enabled_by_flag ? tt('settings.remoteAccess.runOnlyBecausePanel') : undefined} />
            <div className="text-sm">{rich('settings.remoteAccess.listeningCodeNodeNode', { code: <code className="font-mono text-xs">{s.bind}:{s.port}</code>, node: s.tailnet_url ? <Badge tone="plain">{tt('settings.remoteAccess.computerTailscaleNetwork')}</Badge> : loopback ? <Badge tone="plain">{tt('settings.remoteAccess.computerOnly')}</Badge> : <Badge tone="amber">{tt('settings.remoteAccess.reachableFromNetwork')}</Badge> })}</div>
            {s.tailnet_url && <Alert tone="info">{rich('settings.remoteAccess.tailscaleDevicesOpenPanel', { code: <code className="font-mono">{s.tailnet_url}</code>, code2: (c) => <code className="font-mono">{c}</code> })}</Alert>}
            {loopback && !s.tailnet_url && <Alert tone="info">{rich('settings.remoteAccess.openPanelOwnPhone', { code: (c) => <code className="font-mono">{c}</code>, code2: (c) => <code className="font-mono">{c}</code> })}</Alert>}

            <div className="space-y-3">
              <Checkbox label={tt('settings.remoteAccess.letPeopleSignTailscale')} checked={s.tailscale} disabled={save.isPending} onChange={(e) => save.mutate({ tailscale: e.target.checked })}
                hint={tt('settings.remoteAccess.koraAsksTailscaleWho')} />
              {s.tailscale && (
                <div className="space-y-2">
                  <ul className="divide-y divide-border rounded-md border border-border">
                    {Object.entries(s.tailscale_users).map(([l, r]) => (
                      <li key={l} className="flex items-center justify-between gap-3 px-3 py-2 text-sm">
                        <span><strong>{l}</strong> <Badge tone="plain">{r}</Badge></span>
                        <Button size="sm" variant="destructive" onClick={() => setRemoving(l)} aria-label={tt('settings.remoteAccess.removeL', { l })}>{tt('settings.remoteAccess.remove')}</Button>
                      </li>
                    ))}
                  </ul>
                  <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => { e.preventDefault(); const l = login.trim().toLowerCase(); if (!l) return; save.mutate({ tailscale_users: { ...s.tailscale_users, [l]: loginRole } }); setLogin('') }}>
                    <div className="w-56"><Field label={tt('settings.remoteAccess.tailscaleAccount')} hint={tt('settings.remoteAccess.exampleNameExampleCom')}><Input value={login} onChange={(e) => setLogin(e.target.value)} autoComplete="off" /></Field></div>
                    <div className="w-40"><Field label={tt('settings.remoteAccess.role')}><RoleSelect label={tt('settings.remoteAccess.roleAccount')} value={loginRole} onChange={setLoginRole} /></Field></div>
                    <Button type="submit" disabled={!login.trim() || save.isPending}>{tt('settings.remoteAccess.allowAccount')}</Button>
                  </form>
                </div>
              )}
            </div>

            <Users settings={s} />
            <ApiTokens />

            <div className="space-y-2">
              <h3 className="text-sm font-semibold">{tt('settings.remoteAccess.namesPeopleType')}</h3>
              <p className="text-xs text-muted-foreground">{tt('settings.remoteAccess.panelsOwnAddressTailscale')}</p>
              <ul className="space-y-1">
                {s.allowed_hosts.map((h) => (
                  <li key={h} className="flex items-center justify-between gap-3 text-sm"><code className="font-mono text-xs">{h}</code>
                    <Button size="sm" variant="secondary" onClick={() => save.mutate({ allowed_hosts: s.allowed_hosts.filter((x) => x !== h) })} aria-label={tt('settings.remoteAccess.removeH', { h })}>{tt('settings.remoteAccess.remove')}</Button></li>
                ))}
              </ul>
              <form className="flex items-end gap-3" onSubmit={(e) => { e.preventDefault(); const h = host.trim(); if (!h) return; save.mutate({ allowed_hosts: [...s.allowed_hosts, h] }); setHost('') }}>
                <div className="w-64"><Field label={tt('settings.remoteAccess.addName')}><Input value={host} onChange={(e) => setHost(e.target.value)} autoComplete="off" /></Field></div>
                <Button type="submit" disabled={!host.trim() || save.isPending}>{tt('settings.remoteAccess.add')}</Button>
              </form>
            </div>

            <div className="space-y-2">
              <Checkbox label={tt('settings.remoteAccess.requirePasskeyOnetimeCodes')} checked={s.require_strong_auth} disabled={save.isPending} onChange={(e) => save.mutate({ require_strong_auth: e.target.checked })}
                hint={tt('settings.remoteAccess.peopleNoLongerSign')} />
            </div>

            <div className="space-y-2">
              <Checkbox label={tt('settings.remoteAccess.letRemoteAdministratorsDo')} checked={s.allow_remote_destructive} disabled={save.isPending} onChange={(e) => save.mutate({ allow_remote_destructive: e.target.checked })}
                hint={tt('settings.remoteAccess.deletingRemovingServersUninsta')} />
              {s.allow_remote_destructive && <Alert tone="warning">{tt('settings.remoteAccess.remoteAdministratorsAnyoneWho')}</Alert>}
            </div>
            {s.saved_to && <p className="text-xs text-muted-foreground">{tt('settings.remoteAccess.savedSaved', { saved_to: s.saved_to })}</p>}
          </>
        )}
        <ErrorNotice error={save.error ?? q.error} />
        <ConfirmDialog
          open={removing !== null}
          onOpenChange={(o) => { if (!o) setRemoving(null) }}
          title={tt('settings.remoteAccess.removeValue', { value: removing ?? '' })}
          description={tt('settings.remoteAccess.theySignedOutCannot')}
          confirmLabel={tt('settings.remoteAccess.remove')}
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
      <CardHeader><CardTitle>{tt('settings.remoteAccess.yourAccess')}</CardTitle></CardHeader>
      <CardContent className="space-y-3 text-sm">
        <p>{rich('settings.remoteAccess.signedStrongNodeBadge', { strong: <strong>{r.identity.name}</strong>, badge: <Badge tone="plain">{r.role}</Badge>, value: r.remote ? tt('settings.remoteAccess.fromAnotherComputer') : '' })}</p>
        <p className="text-muted-foreground">{tt(ROLE_TEXT[r.role])}.</p>
        {r.remote && !r.destructiveAllowed && <p className="text-muted-foreground">{tt('settings.remoteAccess.deletingServersUninstallingCon')}</p>}
        <Button onClick={() => out.mutate()} disabled={out.isPending}>{tt('settings.remoteAccess.signOut')}</Button>
        <ErrorNotice error={out.error} />
      </CardContent>
    </Card>
  )
}
