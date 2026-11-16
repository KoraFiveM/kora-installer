// /styleguide: every brand token, UI primitive, state and shared component on one page, for styling work.
// Mock builds only (App.tsx loads this behind __KORA_MOCK__, so production builds do not contain it).
import { useState, type ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { ArrowDown, Loader2, Play, Plus, Trash2 } from 'lucide-react'
import { ApiError } from '@/api/client'
import type { ProgressEvent, ServerStatus } from '@/api/types'
import { seedFleet } from '@/api/mock/data'
import { Alert } from '@/components/ui/alert'
import { Badge, type Tone } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox, Radio } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogTrigger } from '@/components/ui/dialog'
import { Input, Label, Select, Textarea } from '@/components/ui/input'
import { Progress } from '@/components/ui/progress'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { ResourceProgress, StepProgress } from '@/components/JobProgress'
import { ServerControls } from '@/components/ServerControls'
import { Skeleton } from '@/components/Skeleton'
import { ModeChip, StatusChip, UpdateChip } from '@/components/Status'

const TOKENS: { name: string; hex: string; cssVar: string; use: string }[] = [
  { name: 'Kora Lime', hex: '#A8FF00', cssVar: '--kora-lime', use: 'primary action, active state (sparingly: one lime element per view)' },
  { name: 'Charcoal', hex: '#1D1E24', cssVar: '--kora-charcoal', use: 'cards, surfaces' },
  { name: 'Ink', hex: '#111315', cssVar: '--kora-ink', use: 'page background' },
  { name: 'Mist', hex: '#BCBDC0', cssVar: '--kora-mist', use: 'secondary text' },
  { name: 'White', hex: '#FFFFFF', cssVar: '--kora-white', use: 'primary text' },
  { name: 'Signal Blue', hex: '#4AA8FF', cssVar: '--kora-blue', use: 'info' },
  { name: 'Success', hex: '#2ECC71', cssVar: '--kora-success', use: 'online, done' },
  { name: 'Amber', hex: '#F5A623', cssVar: '--kora-amber', use: 'restarting, pending' },
  { name: 'Orange', hex: '#E67E22', cssVar: '--kora-orange', use: 'notices' },
  { name: 'Danger', hex: '#E74C3C', cssVar: '--kora-danger', use: 'offline, errors, destructive' },
  { name: 'Steel', hex: '#95A5A6', cssVar: '--kora-steel', use: 'neutral' },
]
const SEMANTIC = ['background', 'foreground', 'card', 'primary', 'primary-foreground', 'secondary', 'muted', 'muted-foreground', 'accent', 'destructive', 'border', 'input', 'ring']

const TYPE_SCALE: [string, string][] = [
  ['text-xs', '12 px / text-xs'],
  ['text-sm', '14 px / text-sm (body in tables and forms)'],
  ['text-base', '16 px / text-base'],
  ['text-lg', '18 px / text-lg'],
  ['text-xl', '20 px / text-xl'],
  ['text-2xl', '24 px / text-2xl (page titles)'],
]

const TONES: Tone[] = ['neutral', 'success', 'amber', 'danger', 'info', 'orange', 'plain']

function Section({ id, title, note, children }: { id: string; title: string; note?: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="space-y-4">
      <div>
        <h2 id={id} className="font-heading text-xl font-bold">{title}</h2>
        {note && <p className="text-sm text-muted-foreground">{note}</p>}
      </div>
      {children}
    </section>
  )
}
const Row = ({ label, children }: { label: string; children: ReactNode }) => (
  <div className="flex flex-wrap items-center gap-3">
    <div className="w-32 shrink-0 text-xs text-muted-foreground">{label}</div>
    <div className="flex flex-wrap items-center gap-3">{children}</div>
  </div>
)

// Tailwind cannot force :hover, so "hover (forced)" repeats what each variant does on hover.
const HOVER: Record<string, string> = { default: 'bg-primary/90', secondary: 'bg-secondary/80', outline: 'bg-accent', ghost: 'bg-accent', destructive: 'bg-destructive/90', link: 'underline' }
const FOCUS = 'outline-2 outline-offset-2 outline-ring'
const VARIANTS = ['default', 'secondary', 'outline', 'ghost', 'destructive', 'link'] as const

function Buttons() {
  return (
    <div className="space-y-3">
      {VARIANTS.map((v) => (
        <Row key={v} label={v}>
          <Button variant={v}>Default</Button>
          <Button variant={v} className={HOVER[v]}>Hover (forced)</Button>
          <Button variant={v} className={FOCUS}>Focus (forced)</Button>
          <Button variant={v} disabled>Disabled</Button>
          <Button variant={v} disabled><Loader2 className="animate-spin" /> Loading</Button>
          <Button variant={v}><Plus /> With icon</Button>
        </Row>
      ))}
      <Row label="sizes">
        <Button size="sm">Small</Button>
        <Button>Default</Button>
        <Button size="lg">Large</Button>
        <Button size="icon" aria-label="Delete"><Trash2 /></Button>
      </Row>
    </div>
  )
}

function Controls() {
  const [on, setOn] = useState(true)
  return (
    <div className="grid gap-6 md:grid-cols-2">
      <div className="space-y-4">
        <Field label="Text input" hint="A hint under the field."><Input placeholder="my-server" /></Field>
        <Field label="Focused (forced)"><Input className={FOCUS} defaultValue="Focus ring" /></Field>
        <Field label="With an error" error="That name is already used."><Input defaultValue="kora-live" /></Field>
        <Field label="Disabled"><Input disabled defaultValue="Cannot change" /></Field>
        <Field label="Select"><Select defaultValue="b"><option value="a">Alpha</option><option value="b">Beta</option></Select></Field>
        <Field label="Textarea"><Textarea placeholder="Tell us what happened" /></Field>
        <div className="space-y-1.5"><Label htmlFor="sg-label">Bare label + input</Label><Input id="sg-label" /></div>
      </div>
      <div className="space-y-3">
        <Checkbox label="Checkbox, checked" hint="With a hint line." checked={on} onChange={(e) => setOn(e.target.checked)} />
        <Checkbox label="Checkbox, unchecked" />
        <Checkbox label="Checkbox, disabled" disabled />
        <Checkbox label="Checkbox, disabled and checked" disabled defaultChecked />
        <Radio name="sg-radio" label="Radio, selected" hint="The border turns lime." defaultChecked />
        <Radio name="sg-radio" label="Radio, other" />
        <Radio name="sg-radio" label="Radio, disabled" disabled />
      </div>
    </div>
  )
}

const STATUSES: ServerStatus[] = ['running', 'stopped', 'missing']

function Chips() {
  return (
    <div className="space-y-3">
      <Row label="Badge tones">{TONES.map((t) => <Badge key={t} tone={t}>{t}</Badge>)}</Row>
      <Row label="With a dot">{TONES.map((t) => <Badge key={t} tone={t} dot>{t}</Badge>)}</Row>
      <Row label="Server status">
        {STATUSES.map((s) => <StatusChip key={s} status={s} />)}
        <StatusChip status="running" pending="Restarting" />
        <StatusChip status="stopped" pending="Starting" />
      </Row>
      <Row label="Update badge">
        <UpdateChip badge={{ kind: 'up_to_date' }} />
        <UpdateChip badge={{ kind: 'updates', count: 1 }} />
        <UpdateChip badge={{ kind: 'updates', count: 12 }} />
        <UpdateChip badge={{ kind: 'needs_attention' }} />
        <UpdateChip badge={{ kind: 'new_release' }} />
        <UpdateChip badge={null} />
      </Row>
      <Row label="Mode"><ModeChip mode="production" /><ModeChip mode="development" /></Row>
    </div>
  )
}

const step = (s: string, title: string, status: ProgressEvent['status'], percent: number, detail = ''): ProgressEvent => ({ step: s, title, status, detail, percent })
const PLANNED = [
  { id: 'prepare', title: 'Checking your computer' },
  { id: 'fx', title: 'Downloading the FiveM server' },
  { id: 'db', title: 'Setting up the database' },
  { id: 'launch', title: 'Starting the server' },
]

function Jobs() {
  const kept = ['The FiveM server files in /srv/fivem/my-server', 'Your answers (use "Try again" to continue)']
  const elevation = new ApiError(403, { code: 'needs_elevation', message: 'Installing the background service needs administrator rights. Run this command as administrator:', command: ['kora', 'install', '--yes', '--name', 'my-server'] })
  return (
    <div className="grid gap-5 lg:grid-cols-2">
      <Card><CardHeader><CardTitle>Pending</CardTitle></CardHeader><CardContent><StepProgress events={[]} planned={PLANNED} /></CardContent></Card>
      <Card>
        <CardHeader><CardTitle>Running at 40%</CardTitle></CardHeader>
        <CardContent><StepProgress planned={PLANNED} events={[step('prepare', 'Checking your computer', 'done', 10), step('fx', 'Downloading the FiveM server', 'running', 40, 'Working on it')]} /></CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>Done (with a skipped step)</CardTitle></CardHeader>
        <CardContent><StepProgress planned={PLANNED} events={[step('prepare', 'Checking your computer', 'done', 25), step('fx', 'Downloading the FiveM server', 'done', 50), step('db', 'Setting up the database', 'skipped', 75, 'Not needed for your choices'), step('launch', 'Starting the server', 'done', 100)]} /></CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>Failed, with what was kept and the log</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <StepProgress planned={PLANNED} events={[step('prepare', 'Checking your computer', 'done', 25), step('fx', 'Downloading the FiveM server', 'done', 50), step('db', 'Setting up the database', 'failed', 60, 'Could not log in to the database')]} />
          <ErrorNotice error={{ code: 'internal', message: 'The database refused the administrator login, so the install stopped before creating the server database.' }} title="The install could not finish" />
          <div className="text-sm"><div className="font-semibold">What was kept</div><ul className="list-disc pl-5 text-muted-foreground">{kept.map((k) => <li key={k}>{k}</li>)}</ul></div>
          <p className="text-sm">Details are in <code className="font-mono text-xs">/srv/fivem/my-server/.kora/install.log</code></p>
        </CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>Needs administrator rights</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <StepProgress planned={PLANNED} events={[step('prepare', 'Checking your computer', 'done', 10)]} />
          <ErrorNotice error={elevation} />
        </CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>Per-resource progress</CardTitle><CardDescription>Updates, add resources, self-update, uninstall</CardDescription></CardHeader>
        <CardContent>
          <ResourceProgress events={[{ type: 'snapshot', id: 'snap-20260310-0400' }, { type: 'resource', id: 'kora_core', state: 'updated' }, { type: 'resource', id: 'kora_inventory', state: 'updating' }, { type: 'health', message: 'Checking the files are healthy' }]} />
        </CardContent>
      </Card>
      <Card className="lg:col-span-2">
        <CardHeader><CardTitle>Progress bar values</CardTitle></CardHeader>
        <CardContent className="space-y-3">{[0, 40, 100].map((v) => <div key={v} className="flex items-center gap-3"><Progress value={v} className="flex-1" /><span className="w-10 text-right text-sm tabular-nums">{v}%</span></div>)}</CardContent>
      </Card>
    </div>
  )
}

function Notices() {
  return (
    <div className="space-y-3">
      <Alert tone="info" title="Information">A neutral message with a title.</Alert>
      <Alert tone="success" title="Success">Updated. Restart the panel to use the new version.</Alert>
      <Alert tone="warning" title="Warning">The server is not running, so there is no live output.</Alert>
      <Alert tone="danger" title="Danger">The database refused that login.</Alert>
      <Alert tone="info">An alert without a title, with a long line to show wrapping: Kora keeps your answers (not your licence key) when an install stops part-way, so you can continue later.</Alert>
      <ErrorNotice error={new ApiError(409, { code: 'conflict', message: 'Port 30120 is already in use by "kora-live". Change the port on the Ports tab first.' })} title="ErrorNotice (API error)" />
      <ErrorNotice error={new ApiError(403, { code: 'needs_elevation', message: 'Changing the automatic updates needs administrator rights. Run the command shown from a terminal opened as administrator, then come back here.', command: ['kora', 'schedule', 'set', '/srv/fivem/kora-dev', 'daily', '--at', '04:00', '--run-as', 'system'] })} />
      <ErrorNotice error={new ApiError(0, { code: 'network', message: 'Could not reach Kora. Check that it is still running, then try again.' })} />
    </div>
  )
}

function EmptyState() {
  return (
    <Card>
      <CardContent className="flex flex-col items-start gap-4 p-8">
        <h2 className="font-heading text-xl font-semibold">No servers yet</h2>
        <p className="max-w-prose text-muted-foreground">Kora can set up a new FiveM server for you in a few minutes: the server files, database and resources. Or add one you already have.</p>
        <Button variant="default"><Plus /> Install your first server</Button>
      </CardContent>
    </Card>
  )
}

function LongTable() {
  const rows = seedFleet().filter((s, i) => i % 3 === 0 || !s.managed || s.name.length >= 32).slice(0, 12)
  return (
    <Card className="overflow-x-auto">
      <table className="w-full min-w-[44rem] text-left text-sm">
        <caption className="sr-only">Sample servers with long names</caption>
        <thead className="text-xs text-muted-foreground"><tr>{['Name', 'Mode', 'Status', 'Game port', 'txAdmin port', 'Updates', 'Actions'].map((h) => <th key={h} scope="col" className="px-4 py-3 font-medium">{h}</th>)}</tr></thead>
        <tbody>
          {rows.map((s) => (
            <tr key={s.name} className="border-t border-border align-middle">
              <td className="px-4 py-3"><a href="#top" className="font-medium hover:underline">{s.name}</a>{!s.managed && <div className="text-xs text-muted-foreground">Added from an existing folder</div>}</td>
              <td className="px-4 py-3"><ModeChip mode={s.mode} /></td>
              <td className="px-4 py-3"><StatusChip status={s.status} /></td>
              <td className="px-4 py-3 font-mono text-xs">{s.ports.game}</td>
              <td className="px-4 py-3 font-mono text-xs">{s.ports.txadmin}</td>
              <td className="px-4 py-3"><UpdateChip badge={s.badge} /></td>
              <td className="px-4 py-3"><ServerControls name={s.name} status={s.status} busy={false} run={() => undefined} /></td>
            </tr>
          ))}
        </tbody>
      </table>
    </Card>
  )
}

const CONSOLE_SAMPLE = [
  'Connected to the kora-live console.',
  '[12:00:01] Started resource kora_core',
  '[12:00:03] Player "Sam" connected (id 12)',
  '[12:00:07] oxmysql: 4 queries in the last minute, slowest 6 ms',
  '> status',
  'Players online: 3/48',
  '[12:00:11] kora_jobs: a very long line that has to wrap rather than overflow the box: payroll run complete for 3 players, 2 skipped because they were off duty, next run in 15 minutes',
  '[12:00:14] Server thread heartbeat ok',
]
function ConsoleSample() {
  return (
    <div className="space-y-3">
      <div className="relative">
        <div role="log" aria-label="Sample console output" tabIndex={0} className="h-64 overflow-y-auto rounded-lg border border-border bg-ink p-3 font-mono text-xs leading-relaxed">
          {CONSOLE_SAMPLE.map((l, i) => <div key={i} className="whitespace-pre-wrap break-words">{l}</div>)}
        </div>
        <Button size="sm" className="absolute bottom-3 right-5"><ArrowDown /> Paused. Jump to latest</Button>
      </div>
      <div className="flex gap-2"><Input aria-label="Sample console command" className="font-mono" placeholder="Type a command, for example status" /><Button variant="default">Send</Button></div>
    </div>
  )
}

function Overlays() {
  const [confirm, setConfirm] = useState<'normal' | 'destructive' | null>(null)
  return (
    <div className="space-y-4">
      <Row label="Dialog">
        <Dialog>
          <DialogTrigger asChild><Button>Open a dialog</Button></DialogTrigger>
          <DialogContent title="Dialog title" description="A short description of what this does."><p className="text-sm">Dialog body.</p></DialogContent>
        </Dialog>
        <Button onClick={() => setConfirm('normal')}>Confirm dialog</Button>
        <Button variant="destructive" onClick={() => setConfirm('destructive')}>Destructive confirm</Button>
      </Row>
      <ConfirmDialog open={confirm === 'normal'} onOpenChange={() => setConfirm(null)} title="Restart kora-live?" description="Everyone playing will be disconnected for a minute." confirmLabel="Restart server" onConfirm={() => setConfirm(null)} />
      <ConfirmDialog open={confirm === 'destructive'} onOpenChange={() => setConfirm(null)} title="Delete kora-old?" description="This deletes the server folder and cannot be undone." confirmLabel="Delete server" destructive onConfirm={() => setConfirm(null)} />
      <Tabs defaultValue="one">
        <TabsList><TabsTrigger value="one">Overview</TabsTrigger><TabsTrigger value="two">Updates</TabsTrigger><TabsTrigger value="three" disabled>Disabled</TabsTrigger></TabsList>
        <TabsContent value="one"><p className="text-sm text-muted-foreground">Tab content one.</p></TabsContent>
        <TabsContent value="two"><p className="text-sm text-muted-foreground">Tab content two.</p></TabsContent>
      </Tabs>
      <Row label="Loading"><Skeleton className="h-10 w-64" /><Skeleton className="h-10 w-40" /></Row>
      <Row label="Copy field"><div className="w-full max-w-xl"><CopyField value="sudo kora schedule set /srv/fivem/kora-dev daily --at 04:00 --run-as system" label="Sample command" /></div></Row>
    </div>
  )
}

export default function Styleguide() {
  return (
    <div id="top" className="mx-auto max-w-6xl space-y-12 p-6">
      <header className="space-y-2">
        <Link to="/" className="text-sm text-info hover:underline">Back to your servers</Link>
        <h1 className="font-heading text-3xl font-bold">Styleguide</h1>
        <p className="text-sm text-muted-foreground">Demo page (mock builds only). Tokens live in <code className="font-mono text-xs">src/index.css</code>; components in <code className="font-mono text-xs">src/components/</code>.</p>
      </header>

      <Section id="sg-colours" title="Brand colours" note="docs/panel-plan.md section 5. Change them in src/index.css (--kora-*).">
        <div className="grid grid-cols-2 gap-3 md:grid-cols-3 lg:grid-cols-4">
          {TOKENS.map((t) => (
            <div key={t.name} className="overflow-hidden rounded-lg border border-border">
              <div className="h-16" style={{ background: `var(${t.cssVar})` }} />
              <div className="space-y-0.5 bg-card p-3 text-xs">
                <div className="text-sm font-semibold">{t.name}</div>
                <div className="font-mono">{t.hex}</div>
                <div className="font-mono text-muted-foreground">{t.cssVar}</div>
                <div className="text-muted-foreground">{t.use}</div>
              </div>
            </div>
          ))}
        </div>
        <h3 className="text-base font-semibold">Semantic variables (shadcn)</h3>
        <div className="grid grid-cols-2 gap-3 md:grid-cols-4 lg:grid-cols-6">
          {SEMANTIC.map((n) => (
            <div key={n} className="flex items-center gap-2 text-xs"><span className="size-6 shrink-0 rounded border border-border" style={{ background: `var(--${n})` }} /><span className="font-mono">--{n}</span></div>
          ))}
        </div>
      </Section>

      <Section id="sg-type" title="Typography" note="Inter Tight for headings, Inter for body, JetBrains Mono for console and code (bundled in src/fonts.css).">
        <div className="grid gap-4 md:grid-cols-3">
          <Card><CardContent className="space-y-1 p-5"><div className="text-xs text-muted-foreground">font-heading: Inter Tight</div><div className="font-heading text-3xl font-bold">Servers, updated.</div><div className="font-heading text-xl font-semibold">Semibold heading</div></CardContent></Card>
          <Card><CardContent className="space-y-1 p-5"><div className="text-xs text-muted-foreground">font-sans: Inter</div><div className="text-base">The quick brown fox jumps over the lazy dog 0123456789</div><div className="text-sm text-muted-foreground">Secondary text in Mist.</div></CardContent></Card>
          <Card><CardContent className="space-y-1 p-5"><div className="text-xs text-muted-foreground">font-mono: JetBrains Mono</div><div className="font-mono text-sm">ensure kora_core</div><div className="font-mono text-xs">30120 / 40120 {'{ } [ ] => 0O1lI'}</div></CardContent></Card>
        </div>
        <div className="space-y-2">
          {TYPE_SCALE.map(([c, label]) => <div key={c} className="flex items-baseline gap-4"><div className="w-56 shrink-0 text-xs text-muted-foreground">{label}</div><div className={c}>The quick brown fox jumps over the lazy dog</div></div>)}
          <div className="flex items-baseline gap-4"><div className="w-56 shrink-0 text-xs text-muted-foreground">Page title: font-heading text-2xl font-bold</div><h3 className="font-heading text-2xl font-bold">Your servers</h3></div>
          <div className="flex items-baseline gap-4"><div className="w-56 shrink-0 text-xs text-muted-foreground">Weights</div><div className="space-x-4"><span className="font-normal">400</span><span className="font-medium">500</span><span className="font-semibold">600</span><span className="font-bold">700</span></div></div>
        </div>
      </Section>

      <Section id="sg-buttons" title="Buttons" note="Variants x states. Hover and focus are forced with classes (Tailwind cannot force them).">
        <Buttons />
        <Row label="In a row of actions"><Button variant="default"><Play /> Start</Button><Button>Stop</Button><Button variant="destructive"><Trash2 /> Delete server</Button></Row>
      </Section>
      <Section id="sg-controls" title="Form controls"><Controls /></Section>
      <Section id="sg-chips" title="Badges and status chips"><Chips /></Section>
      <Section id="sg-notices" title="Alerts and error notices"><Notices /></Section>
      <Section id="sg-jobs" title="Job progress" note="StepProgress (installs) and ResourceProgress (updates, add resources) in each state."><Jobs /></Section>
      <Section id="sg-cards" title="Cards and empty state">
        <div className="grid gap-4 md:grid-cols-2">
          <Card><CardHeader><CardTitle>Card title</CardTitle><CardDescription>Card description in Mist.</CardDescription></CardHeader><CardContent><p className="text-sm">Card content.</p></CardContent></Card>
          <Card className="border-destructive/50"><CardHeader><CardTitle>Danger card</CardTitle></CardHeader><CardContent><Button variant="destructive">Uninstall</Button></CardContent></Card>
        </div>
        <EmptyState />
      </Section>
      <Section id="sg-table" title="Table with long content" note="32-character names, high ports and an added server. Resize the window to check overflow."><LongTable /></Section>
      <Section id="sg-console" title="Console view"><ConsoleSample /></Section>
      <Section id="sg-overlays" title="Dialogs, tabs, loading"><Overlays /></Section>
    </div>
  )
}
