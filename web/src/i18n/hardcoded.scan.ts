/**
 * Finds user-facing text written straight into the source instead of the catalogue. Used by hardcoded.test.ts (which
 * fails when a new one appears) and by `scripts/i18n-todo.mjs`-style listings while extracting.
 *
 * It reads the code with the TypeScript compiler API (already installed), so it sees the real syntax, not lines.
 * What counts as user-facing text:
 *   - text between JSX tags, and string / template literals inside `{}` between JSX tags;
 *   - attributes that carry words for people: aria-label, title, placeholder, alt, label, hint, ... (USER_ATTRS), and any
 *     other attribute whose value reads like a sentence;
 *   - elsewhere (lib, hooks, helpers): a string or template literal that reads like a sentence (starts with a capital,
 *     has several words) and is not an import path, a type, a case label or a log line.
 * A deliberate exception (a command shown verbatim, a technical token): `// i18n-ignore: why` on the line or the line above.
 */
import ts from 'typescript'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

export interface Finding {
  file: string
  line: number
  text: string
  kind: 'jsx-text' | 'jsx-attr' | 'jsx-expr' | 'string'
}

const USER_ATTRS = new Set([
  'aria-label', 'aria-description', 'aria-roledescription', 'aria-placeholder', 'title', 'placeholder', 'alt', 'label', 'hint', 'description', 'text', 'caption', 'summary',
  'heading', 'tooltip', 'confirmLabel', 'cancelLabel', 'emptyText', 'message', 'error', 'help', 'legend',
])
const NOT_TEXT_ATTRS = new Set([
  'className', 'id', 'key', 'type', 'href', 'to', 'name', 'value', 'defaultValue', 'role', 'rel', 'target', 'autoComplete', 'inputMode', 'htmlFor', 'variant', 'tone', 'size', 'd', 'fill',
  'stroke', 'viewBox', 'xmlns', 'style', 'aria-live', 'aria-hidden', 'aria-busy', 'aria-current', 'aria-controls', 'aria-describedby', 'aria-labelledby', 'aria-atomic', 'aria-sort', 'accept',
  'src', 'srcSet', 'min', 'max', 'step', 'pattern', 'lang', 'dir', 'spellCheck', 'data-testid', 'data-state', 'width', 'height', 'viewport', 'as', 'cx', 'cy', 'r', 'points', 'transform',
])

/** Names that are not translated: the product's and its neighbours'. */
const PROPER = /^(Kora|txAdmin|FiveM|GitHub|Cfx\.re|Tailscale|Discord|Windows|Linux|macOS|HTTPS|HTTP|TLS|UPnP|PCP|NAT-PMP|SQLite|MariaDB|MySQL|API|URL|IP|OK|ID|CPU|RAM|GB|MB|KB|TCP|UDP|CLI|PR|SHA|Git|PowerShell|Docker|Cloudflare|Let's Encrypt)$/

function words(s: string): string[] {
  return s.match(/[A-Za-z][A-Za-z'’-]*/g) ?? []
}

/** Is this text something a person reads (as opposed to a code, a class list, a path, a number)? */
export function looksLikeText(s: string, strict: boolean): boolean {
  const t = s.trim()
  if (!t) return false
  const w = words(t)
  if (w.length === 0) return false
  if (w.every((x) => PROPER.test(x))) return false
  if (/^(https?:\/\/|\/|\.|@|#|[A-Za-z]:\\)/.test(t) && !/\s/.test(t)) return false
  if (!strict) return true
  // Outside JSX: only sentences. Capital first, a space, a few words, or ends like a sentence.
  if (/^[a-z]/.test(t)) return false
  if (/^[A-Z][A-Z0-9_ -]+$/.test(t)) return false
  if (/^[A-Z][A-Za-z0-9]*$/.test(t)) return w.length === 1 && t.length > 3 && !/[0-9]/.test(t) && false
  return w.length >= 2 && /\s/.test(t)
}

export function scanSource(file: string, source: string): Finding[] {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, file.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
  const lines = source.split('\n')
  const out: Finding[] = []
  const ignored = (line0: number) => /i18n-ignore/.test(lines[line0] ?? '') || /i18n-ignore/.test(lines[line0 - 1] ?? '')
  const add = (node: ts.Node, text: string, kind: Finding['kind']) => {
    const line0 = sf.getLineAndCharacterOfPosition(node.getStart(sf)).line
    if (ignored(line0)) return
    out.push({ file, line: line0 + 1, text: text.replace(/\s+/g, ' ').trim().slice(0, 90), kind })
  }

  const inJsxChild = (n: ts.Node) => ts.isJsxExpression(n.parent) && (ts.isJsxElement(n.parent.parent) || ts.isJsxFragment(n.parent.parent))
  const literalText = (n: ts.Node): string | null => (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n) ? n.text : ts.isTemplateExpression(n) ? n.head.text + n.templateSpans.map((s) => s.literal.text).join(' ') : null)

  /** String-ish leaves of an expression that ends up on screen: `cond ? 'a' : 'b'`, `x ?? 'a'`, `(a && 'b')`, templates. */
  const leaves = (e: ts.Expression, fn: (l: ts.Expression) => void) => {
    if (ts.isParenthesizedExpression(e)) return leaves(e.expression, fn)
    if (ts.isConditionalExpression(e)) {
      leaves(e.whenTrue, fn)
      leaves(e.whenFalse, fn)
      return
    }
    if (ts.isBinaryExpression(e) && [ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken].includes(e.operatorToken.kind)) {
      leaves(e.left, fn)
      leaves(e.right, fn)
      return
    }
    if (literalText(e) !== null) fn(e)
  }

  const visit = (n: ts.Node) => {
    if (ts.isImportDeclaration(n) || ts.isExportDeclaration(n) || ts.isTypeNode(n) || ts.isInterfaceDeclaration(n) || ts.isTypeAliasDeclaration(n)) return
    if (ts.isJsxText(n)) {
      const t = n.text
      if (looksLikeText(t, false)) add(n, t, 'jsx-text')
    } else if (ts.isJsxAttribute(n)) {
      const name = n.name.getText(sf)
      const init = n.initializer
      const check = (e: ts.Node, kind: Finding['kind']) => {
        const text = literalText(e)
        if (text === null) return
        if (USER_ATTRS.has(name) ? looksLikeText(text, false) : !NOT_TEXT_ATTRS.has(name) && !name.startsWith('data-') && looksLikeText(text, true)) add(e, text, kind)
      }
      if (init && (ts.isStringLiteral(init) || ts.isNoSubstitutionTemplateLiteral(init))) check(init, 'jsx-attr')
      else if (init && ts.isJsxExpression(init) && init.expression) leaves(init.expression, (l) => check(l, 'jsx-attr'))
    } else if (ts.isJsxExpression(n) && inJsxChild(n) && n.expression) {
      leaves(n.expression, (l) => {
        const text = literalText(l)!
        if (looksLikeText(text, false)) add(l, text, 'jsx-expr')
      })
    } else if (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n) || ts.isTemplateExpression(n)) {
      const p = n.parent
      const skip =
        ts.isCaseClause(p) ||
        ts.isLiteralTypeNode(p) ||
        ts.isPropertyAssignment(p) && p.name === n ||
        (ts.isBinaryExpression(p) && [ts.SyntaxKind.EqualsEqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsEqualsToken].includes(p.operatorToken.kind)) ||
        (ts.isElementAccessExpression(p) && p.argumentExpression === n) ||
        (ts.isCallExpression(p) && /^(console\.|(\w+\.)?(getItem|setItem|removeItem|querySelector|querySelectorAll|getElementById|addEventListener|removeEventListener|matchMedia|postMessage|fetch|import|require|include|includes|startsWith|endsWith|test|match|split|replace|replaceAll|join)$|cn$|clsx$|cva$|t$|rich$)/.test(p.expression.getText(sf))) ||
        (ts.isJsxExpression(p) && !inJsxChild(n))
      const text = literalText(n)
      if (!skip && text !== null && !inJsx(n) && looksLikeText(text, true)) add(n, text, 'string')
    }
    ts.forEachChild(n, visit)
  }
  const inJsx = (n: ts.Node): boolean => {
    for (let p: ts.Node | undefined = n.parent; p; p = p.parent) {
      if (ts.isJsxAttribute(p) || ts.isJsxExpression(p)) return true
      if (ts.isFunctionLike(p) || ts.isSourceFile(p)) return false
    }
    return false
  }
  visit(sf)
  return out
}

/** Every non-test source file of the app that should be free of hard-coded text. */
export function sourceFiles(root: string): string[] {
  const out: string[] = []
  const walk = (dir: string) => {
    for (const e of readdirSync(dir)) {
      const p = join(dir, e)
      if (statSync(p).isDirectory()) {
        if (['mock', 'generated', 'test', 'i18n'].includes(e) || (e === 'Styleguide.tsx')) continue
        walk(p)
      } else if (/\.(ts|tsx)$/.test(e) && !/\.(test|spec)\.|\.d\.ts$/.test(e)) out.push(p)
    }
  }
  walk(root)
  return out.sort()
}

export function scanAll(srcRoot: string): Finding[] {
  const out: Finding[] = []
  for (const f of sourceFiles(srcRoot)) out.push(...scanSource(relative(srcRoot, f), readFileSync(f, 'utf8')))
  return out
}
