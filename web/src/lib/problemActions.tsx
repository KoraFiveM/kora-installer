/**
 * What the buttons under an error do. A Problem (see `Problem` in api/types.ts) lists actions as
 * data; this file turns them into buttons.
 *
 * - Pages pass handlers for the kinds that depend on where the error is shown (the install wizard
 *   handles `retry_step`, `skip_step`, `change_answers`, ...), as `<ErrorNotice actions={...}>`.
 * - Handlers that work anywhere (for example `install_admin_service` or `install_tool`)
 *   are registered for the whole app with `<ProblemActions handlers={...}>` around it. Nested
 *   providers merge; the innermost handler for a kind wins, and a page's own prop wins over both.
 * - `show_command`, `open_link`, `open_log` and `copy_details` are drawn by ErrorNotice itself and
 *   need no handler.
 * - An action without a handler is not shown. So the server may offer an action before any page
 *   can do it, and an older page simply leaves out actions it does not know.
 *
 * To add a kind: add it to `ProblemAction` in api/types.ts (same name as the Rust `Action`
 * variant, snake_case), give it a label in `actionLabel` below, and register a handler.
 */
import { createContext, useContext, useMemo, type ReactNode } from 'react'
import { ApiError } from '@/api/client'
import type { JobError, Problem, ProblemAction, ProblemActionKind } from '@/api/types'

export type ActionOf<K extends ProblemActionKind> = Extract<ProblemAction, { kind: K }>
/**
 * Given to every handler: where the button was pressed and how to carry on.
 * `afterFix` is what to do once a fix action (install a tool, sign in, start the database) has worked: the
 * page that showed the error says how to carry on (retry the failed step, reload). Absent when the error
 * sits on something that cannot simply be retried.
 */
export interface ActionContext {
  /** The problem the button belongs to. */
  problem?: NoticeProblem
  /** Repeat what failed: the page's own "Try again", else the same request sent again. */
  retry?: () => unknown
  afterFix?: () => void | Promise<void>
}
export type ActionHandlers = { [K in ProblemActionKind]?: (a: ActionOf<K>, ctx: ActionContext) => void | Promise<void> }

/** Kinds ErrorNotice draws by itself (a copy field, a link, the log, a copy button). */
export const BUILT_IN: readonly ProblemActionKind[] = ['show_command', 'open_link', 'open_log', 'copy_details']
// `install_admin_service` is a button only where a handler can launch the installer (Windows); elsewhere
// ErrorNotice explains it and shows the command (`kora service install`).

const KNOWN: readonly ProblemActionKind[] = [
  'retry', 'retry_step', 'skip_step', 'change_answers', 'sign_in_github', 'install_tool', 'install_admin_service',
  'continue_without_admin', 'show_command', 'open_link', 'open_log', 'copy_details', 'start_database',
]

/** Wizard pages that `change_answers` can open, as the user sees them. */
const SECTION_LABEL: Record<string, string> = {
  licence: 'the licence key',
  mode: 'the kind of server',
  name: 'the name or folder',
  ports: 'the ports',
  build: 'the server build',
  resources: 'the resources',
  database: 'the database choice',
  options: 'the options',
  cfx: 'the txAdmin sign-in',
  github: 'the GitHub account',
}

const TOOL_LABEL: Record<string, string> = { git: 'Git', gh: 'the GitHub CLI', mariadb: 'MariaDB' }

/** Codes of "this needs administrator rights" problems: the administrator service is missing or not turned on for you. */
export function isAdminCode(code: string): boolean {
  return code === 'needs_elevation' || code.startsWith('service.')
}

export function actionLabel(a: ProblemAction): string {
  switch (a.kind) {
    case 'retry':
      return 'Try again'
    case 'retry_step':
      return 'Try this step again'
    case 'skip_step':
      return 'Skip this step'
    case 'change_answers':
      return a.section && SECTION_LABEL[a.section] ? `Change ${SECTION_LABEL[a.section]}` : 'Change my answers'
    case 'sign_in_github':
      return a.scopes?.length ? 'Sign in to GitHub again' : 'Sign in to GitHub'
    case 'install_tool':
      return `Install ${TOOL_LABEL[a.tool] ?? 'it'} for me`
    case 'install_admin_service':
      return "Install Kora's administrator service"
    case 'continue_without_admin':
      return 'Continue without the administrator parts'
    case 'show_command':
      return a.elevated ? 'Command to run as administrator' : 'Command to run'
    case 'open_link':
      return `Open ${hostOf(a.url)}`
    case 'open_log':
      return 'Show the log'
    case 'copy_details':
      return 'Copy details'
    case 'start_database':
      return 'Start it for me'
    case 'unknown':
    default:
      // A kind from a newer Kora (or the Rust `Action::Unknown`): never shown, see `hasHandler`.
      return ''
  }
}

function hostOf(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return 'the page'
  }
}

/** A problem as ErrorNotice reads it, from anything a page may hold as an error. */
export interface NoticeProblem extends Problem {
  actions: ProblemAction[]
  /** `needs_elevation`: the command to run (older answers only have this, not a `show_command`). */
  command?: string[]
  /** HTTP status of an API error; 0 when Kora did not answer at all. */
  status?: number
  /** An API error: send the same request again (see `ApiError.replay`). */
  replay?: () => Promise<unknown>
}

const isAction = (a: unknown): a is ProblemAction => typeof a === 'object' && a !== null && KNOWN.includes((a as { kind: ProblemActionKind }).kind)

export function toProblem(e: unknown): NoticeProblem | null {
  if (!e) return null
  if (e instanceof ApiError) return { code: e.code, message: e.message, details: e.details, actions: (e.actions ?? []).filter(isAction), command: e.command, status: e.status, replay: e.replay }
  if (typeof e === 'string') return { code: 'failed', message: e, actions: [] }
  if (typeof e === 'object' && 'message' in e) {
    const j = e as JobError
    return { code: j.code ?? 'failed', message: j.message, details: j.details, actions: (Array.isArray(j.actions) ? j.actions : []).filter(isAction), command: j.command }
  }
  return { code: 'failed', message: 'Something went wrong. Please try again.', actions: [] }
}

/** The text "Copy details" puts on the clipboard: everything someone helping would ask for. */
export function detailsText(p: Problem): string {
  return [p.message, p.details, `(code: ${p.code})`].filter(Boolean).join('\n\n')
}

const Ctx = createContext<ActionHandlers>({})

/** Register handlers for every ErrorNotice inside (merged with the ones from outer providers). */
export function ProblemActions({ handlers, children }: { handlers: ActionHandlers; children: ReactNode }) {
  const outer = useContext(Ctx)
  const merged = useMemo(() => ({ ...outer, ...handlers }), [outer, handlers])
  return <Ctx.Provider value={merged}>{children}</Ctx.Provider>
}

export function useProblemHandlers(own?: ActionHandlers): ActionHandlers {
  const ctx = useContext(Ctx)
  return useMemo(() => ({ ...ctx, ...own }), [ctx, own])
}

/** Run the handler of `a` (the caller checked that there is one). */
export function runAction(handlers: ActionHandlers, a: ProblemAction, ctx: ActionContext = {}): void | Promise<void> {
  const h = handlers[a.kind] as ((x: ProblemAction, c: ActionContext) => void | Promise<void>) | undefined
  return h?.(a, ctx)
}

/** The actions that become buttons: those with a handler, built-in ones excluded. */
export function buttonActions(p: NoticeProblem, handlers: ActionHandlers): ProblemAction[] {
  return p.actions.filter((a) => !BUILT_IN.includes(a.kind) && typeof handlers[a.kind] === 'function')
}
