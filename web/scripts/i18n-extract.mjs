// One-off migration tool: moves hard-coded text of the files given on the command line into the English catalogue.
//   node scripts/i18n-extract.mjs <area> <file> [<file> ...]      (paths relative to web/)
// It rewrites each file in place (`t('id')`, `t('id', { n })`, `rich('id', { code: (c) => <code>{c}</code> })`), adds the
// messages to src/i18n/en/<area>.ts and the imports. Anything it is not sure about it leaves alone: the scan test
// (src/i18n/hardcoded.test.ts) lists what remains. Review the diff.
import ts from 'typescript'
import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { basename } from 'node:path'

const [area, ...files] = process.argv.slice(2)
if (!area || files.length === 0) {
  console.error('usage: node scripts/i18n-extract.mjs <area> <file>...')
  process.exit(2)
}
const MODULE = !!process.env.MODULE // module-level strings (tables of labels): `msg('id')` instead of `t('id')`
const catFile = `src/i18n/en/${area}.ts`

// ---- the catalogue file ------------------------------------------------------------------------------------------
function loadCatalogue() {
  if (!existsSync(catFile)) {
    return { text: `import { register } from '../index'\n\nconst messages = {\n} as const\n\nregister('en', messages)\nexport default messages\n`, entries: new Map() }
  }
  const text = readFileSync(catFile, 'utf8')
  const entries = new Map()
  for (const m of text.matchAll(/^ {2}'([^']+)':\s*(.*),$/gm)) entries.set(m[1], m[2])
  return { text, entries }
}
const cat = loadCatalogue()
const added = []

const quote = (s) => (s.includes("'") && !s.includes('"') ? `"${s.replace(/\\/g, '\\\\')}"` : `'${s.replace(/\\/g, '\\\\').replace(/'/g, "\\'")}'`)
const icuEscape = (s) => s.replace(/\\/g, '\\\\').replace(/[{}<]/g, (c) => '\\' + c)

const ENT = { '&apos;': "'", '&quot;': '"', '&amp;': '&', '&lt;': '<', '&gt;': '>', '&nbsp;': ' ', '&middot;': '·', '&hellip;': '…', '&rsquo;': '’', '&lsquo;': '‘', '&ndash;': '–', '&mdash;': '—', '&rarr;': '→', '&larr;': '←', '&times;': '×', '&#39;': "'" }
const decode = (s) => s.replace(/&#?\w+;/g, (m) => ENT[m] ?? m)

/** React's rule for JSX text: trim each line, drop empty lines, join with one space. */
function cleanJsxText(raw) {
  const lines = raw.split(/\r\n|\n|\r/)
  let out = ''
  const last = lines.length - 1
  lines.forEach((line, i) => {
    let l = line.replace(/\t/g, ' ')
    if (i !== 0) l = l.replace(/^[ ]+/, '')
    if (i !== last) l = l.replace(/[ ]+$/, '')
    if (l) {
      if (i !== last) l += ' '
      out += l
    }
  })
  return decode(out)
}

const words = (s) => s.match(/[A-Za-z][A-Za-z'’-]*/g) ?? []
const PROPER = /^(Kora|txAdmin|FiveM|GitHub|Cfx\.re|Tailscale|Discord|Windows|Linux|macOS|HTTPS|HTTP|TLS|UPnP|PCP|NAT-PMP|SQLite|MariaDB|MySQL|API|URL|IP|OK|ID|CPU|RAM|GB|MB|KB|TCP|UDP|CLI|PR|SHA|Git|PowerShell|Docker|Cloudflare)$/
const hasText = (s) => {
  const w = words(s)
  return w.length > 0 && !w.every((x) => PROPER.test(x))
}
const endsInValuePosition = (n) => {
  let cur = n
  for (let p = cur.parent; p; cur = p, p = p.parent) {
    if (ts.isParenthesizedExpression(p) || ts.isAsExpression(p)) continue
    if (ts.isConditionalExpression(p) && p.condition !== cur) continue
    if (ts.isBinaryExpression(p) && [ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken, ts.SyntaxKind.AmpersandAmpersandToken].includes(p.operatorToken.kind)) continue
    return (ts.isPropertyAssignment(p) && p.initializer === cur) || ts.isReturnStatement(p) || (ts.isArrowFunction(p) && p.body === cur) || (ts.isVariableDeclaration(p) && p.initializer === cur)
  }
  return false
}
const tableValue = (t) => /^[A-Z][a-z]+( [A-Za-z']+)*[.!?]?$/.test(t.trim()) && !PROPER.test(t.trim())
const proseString = (t) => {
  const s = t.trim()
  if (!hasText(s)) return false
  if (/^[a-z]/.test(s)) return false
  if (/^[A-Z][A-Z0-9_ -]+$/.test(s)) return false
  return words(s).length >= 2 && /\s/.test(s)
}

const USER_ATTRS = new Set(['aria-label', 'aria-description', 'title', 'placeholder', 'alt', 'label', 'hint', 'description', 'text', 'caption', 'summary', 'heading', 'tooltip', 'confirmLabel', 'cancelLabel', 'emptyText', 'message', 'error', 'help', 'legend'])
const NOT_TEXT_ATTRS = new Set(['className', 'id', 'key', 'type', 'href', 'to', 'name', 'value', 'defaultValue', 'role', 'rel', 'target', 'autoComplete', 'inputMode', 'htmlFor', 'variant', 'tone', 'size', 'd', 'fill', 'stroke', 'viewBox', 'xmlns', 'style', 'aria-live', 'aria-hidden', 'aria-busy', 'aria-current', 'aria-controls', 'aria-describedby', 'aria-labelledby', 'aria-atomic', 'aria-sort', 'accept', 'src', 'srcSet', 'min', 'max', 'step', 'pattern', 'lang', 'dir', 'spellCheck', 'data-testid', 'data-state', 'width', 'height', 'as', 'cx', 'cy', 'r', 'points', 'transform'])

const STOP = new Set(['the', 'a', 'an', 'to', 'of', 'is', 'are', 'it', 'this', 'that', 'and', 'or', 'in', 'on', 'for', 'with', 'be', 'your', 'you', 'can', 'will', 'was', 'has', 'have', 'as', 'at', 'by', 'if', 'so', 'then', 'not'])
const camel = (s) => {
  const all = words(s)
  const w = (all.filter((x) => !STOP.has(x.toLowerCase())).length >= 2 ? all.filter((x) => !STOP.has(x.toLowerCase())) : all).slice(0, 4).map((x) => x.replace(/['’-]/g, '').toLowerCase())
  const joined = w.map((x, i) => (i ? x[0].toUpperCase() + x.slice(1) : x)).join('')
  return (joined || 'text').slice(0, 30)
}

for (const file of files) {
  const src = readFileSync(file, 'utf8')
  const sf = ts.createSourceFile(file, src, ts.ScriptTarget.Latest, true, file.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
  const fileKey = basename(file).replace(/\.(tsx?|ts)$/, '').replace(/^./, (c) => c.toLowerCase()).replace(/[^A-Za-z0-9]/g, '')
  let edits = [] // { s, e, text }
  let usesT = false
  let usesRich = false
  let usesMsg = false
  const usesTName = (() => {
    let found = false
    const v = (n) => {
      if (ts.isIdentifier(n) && n.text === 't') found = true
      ts.forEachChild(n, v)
    }
    v(sf)
    return found
  })()
  const T = usesTName ? 'tt' : 't'

  /** The (edited) source of a node, with the inner edits applied and consumed. */
  const take = (s, e) => {
    const inner = edits.filter((x) => x.s >= s && x.e <= e).sort((a, b) => a.s - b.s)
    edits = edits.filter((x) => !(x.s >= s && x.e <= e))
    let out = ''
    let pos = s
    for (const x of inner) {
      out += src.slice(pos, x.s) + x.text
      pos = x.e
    }
    return out + src.slice(pos, e)
  }

  const idFor = (text, params) => {
    const sig = text
    const base = `${area}.${fileKey}.${camel(text)}`
    let id = base
    let n = 1
    for (;;) {
      const prev = cat.entries.get(id)
      if (prev === undefined) break
      if (prev === quote(sig)) return id
      n += 1
      id = `${base}${n}`
    }
    cat.entries.set(id, quote(sig))
    added.push([id, quote(sig)])
    void params
    return id
  }

  /** Make a unique argument name. */
  const paramName = (used, hint) => {
    let name = hint.replace(/[^A-Za-z0-9_]/g, '') || 'value'
    if (!/^[A-Za-z_]/.test(name)) name = 'value'
    let n = name
    let i = 2
    while (used.has(n)) n = `${name}${i++}`
    used.add(n)
    return n
  }
  const hintOf = (e) => {
    if (ts.isIdentifier(e)) return e.text
    if (ts.isPropertyAccessExpression(e)) return e.name.text === 'length' ? 'count' : e.name.text
    if (ts.isCallExpression(e)) return hintOf(e.expression)
    if (ts.isNonNullExpression(e) || ts.isParenthesizedExpression(e)) return hintOf(e.expression)
    if (ts.isElementAccessExpression(e)) return hintOf(e.expression)
    return 'value'
  }
  const containsJsx = (n) => {
    let f = false
    const v = (x) => {
      if (ts.isJsxElement(x) || ts.isJsxSelfClosingElement(x) || ts.isJsxFragment(x)) f = true
      ts.forEachChild(x, v)
    }
    v(n)
    return f
  }

  /** Is the node (an element / expression) understood well enough to be turned into a message part? Builds segments or returns null. */
  const segmentsOf = (children, used, ctx) => {
    const segs = [] // { text } | { param, code } | { tag, code(attrs text), inner: segs } | { node, code }
    for (const c of children) {
      if (ts.isJsxText(c)) {
        const t = cleanJsxText(c.text)
        if (t) segs.push({ text: t })
      } else if (ts.isJsxExpression(c)) {
        const ex = c.expression
        if (!ex) continue
        if (ts.isStringLiteral(ex) || ts.isNoSubstitutionTemplateLiteral(ex)) {
          segs.push({ text: ex.text })
          edits = edits.filter((x) => !(x.s >= ex.getStart(sf) && x.e <= ex.end))
        } else if (containsJsx(ex)) {
          const nm = paramName(used, 'node')
          segs.push({ node: nm, code: take(ex.getStart(sf), ex.end) })
        } else {
          const nm = paramName(used, hintOf(ex))
          segs.push({ param: nm, code: take(ex.getStart(sf), ex.end) })
        }
      } else if (ts.isJsxElement(c)) {
        const open = c.openingElement
        const tag = open.tagName.getText(sf)
        const inner = segmentsOf(c.children, used, ctx)
        if (inner === null) return null
        const hasWords = inner.some((x) => (x.text !== undefined && hasText(x.text)) || x.tag)
        if (!hasWords) {
          // An element with no words of its own (an icon wrapper, a styled count): one node.
          const nm = paramName(used, tag.replace(/\./g, '').replace(/^./, (x) => x.toLowerCase()))
          segs.push({ node: nm, code: take(c.getStart(sf), c.end) })
        } else {
          const nm = paramName(used, { code: 'code', strong: 'b', b: 'b', em: 'i', i: 'i', a: 'link', Link: 'link', span: 'span', kbd: 'kbd', small: 'small' }[tag] ?? tag.replace(/\./g, '').replace(/^./, (x) => x.toLowerCase()))
          // the opening tag as written (with its attributes, edited), the closing tag
          const openText = take(open.getStart(sf), open.end)
          segs.push({ tag: nm, openText, tagName: tag, inner })
        }
      } else if (ts.isJsxSelfClosingElement(c)) {
        const tag = c.tagName.getText(sf)
        const nm = paramName(used, tag.replace(/\./g, '').replace(/^./, (x) => x.toLowerCase()))
        segs.push({ node: nm, code: take(c.getStart(sf), c.end) })
      } else {
        return null
      }
    }
    return segs
  }

  const render = (segs, params, tags) => {
    let msg = ''
    for (const s of segs) {
      if (s.text !== undefined) msg += icuEscape(s.text)
      else if (s.param) {
        msg += `{${s.param}}`
        params.push([s.param, s.code])
      } else if (s.node) {
        msg += `{${s.node}, node}`
        params.push([s.node, s.code])
      } else if (s.tag) {
        msg += `<${s.tag}>${render(s.inner, params, tags)}</${s.tag}>`
        tags.push(s)
      }
    }
    return msg
  }

  const paramsObject = (params, tags) => {
    const parts = params.map(([n, code]) => (code === n ? n : `${n}: ${code}`))
    for (const t of tags) {
      const open = t.openText
      const name = t.tagName
      parts.push(`${t.tag}: (c) => ${open}{c}</${name}>`)
    }
    return parts.length ? `{ ${parts.join(', ')} }` : ''
  }

  // ---- visit, children first -------------------------------------------------------------------------------------
  const inFunction = (n) => {
    for (let p = n.parent; p; p = p.parent) if (ts.isFunctionLike(p)) return true
    return false
  }
  const literalCall = (text, spans, node) => {
    // a string or template literal -> t('id', { params })
    let msg
    const params = []
    const used = new Set()
    if (spans === null) msg = icuEscape(text)
    else {
      msg = icuEscape(node.head.text)
      for (const sp of node.templateSpans) {
        const nm = paramName(used, hintOf(sp.expression))
        msg += `{${nm}}` + icuEscape(sp.literal.text)
        params.push([nm, take(sp.expression.getStart(sf), sp.expression.end)])
      }
    }
    const id = idFor(msg, params)
    if (MODULE) {
      usesMsg = true
      return `msg('${id}')`
    }
    usesT = true
    return `${T}('${id}'${params.length ? `, { ${params.map(([n, c]) => (c === n ? n : `${n}: ${c}`)).join(', ')} }` : ''})`
  }
  const litOf = (n) => (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n) ? n.text : ts.isTemplateExpression(n) ? n.head.text + n.templateSpans.map((s) => s.literal.text).join(' ') : null)
  const leaves = (e, fn) => {
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
    if (litOf(e) !== null) fn(e)
  }
  const replaceLiteral = (l, asJsxAttrString = false) => {
    const text = litOf(l)
    const call = literalCall(text, ts.isTemplateExpression(l) ? [] : null, l)
    edits.push({ s: l.getStart(sf), e: l.end, text: asJsxAttrString ? `{${call}}` : call })
  }

  const visit = (n) => {
    if (ts.isImportDeclaration(n) || ts.isExportDeclaration(n) || ts.isTypeNode(n) || ts.isInterfaceDeclaration(n) || ts.isTypeAliasDeclaration(n)) return
    // children first
    ts.forEachChild(n, visit)

    if (ts.isJsxAttribute(n) && n.initializer) {
      const name = n.name.getText(sf)
      const user = USER_ATTRS.has(name)
      const judge = (text) => (user ? hasText(text) : !NOT_TEXT_ATTRS.has(name) && !name.startsWith('data-') && proseString(text))
      const init = n.initializer
      if (ts.isStringLiteral(init)) {
        if (judge(decode(init.text)) && inFunction(n)) {
          const call = literalCall(decode(init.text), null, init)
          edits.push({ s: init.getStart(sf), e: init.end, text: `{${call}}` })
        }
      } else if (ts.isJsxExpression(init) && init.expression && inFunction(n)) {
        leaves(init.expression, (l) => {
          const text = litOf(l)
          if (judge(text)) replaceLiteral(l)
        })
      }
      return
    }
    // a plain string / template leaf outside JSX
    if ((ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n) || ts.isTemplateExpression(n)) && inFunction(n) === !MODULE) {
      const p = n.parent
      const inJsxPos = ts.isJsxAttribute(p) || ts.isJsxExpression(p)
      const text = litOf(n)
      const skip =
        inJsxPos ||
        ts.isCaseClause(p) ||
        ts.isLiteralTypeNode(p) ||
        (ts.isPropertyAssignment(p) && p.name === n) ||
        (ts.isBinaryExpression(p) && [ts.SyntaxKind.EqualsEqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsEqualsToken].includes(p.operatorToken.kind)) ||
        (ts.isElementAccessExpression(p) && p.argumentExpression === n) ||
        (ts.isCallExpression(p) && /^(console\.|(\w+\.)?(getItem|setItem|removeItem|querySelector|querySelectorAll|getElementById|addEventListener|removeEventListener|matchMedia|postMessage|fetch|import|require|include|includes|startsWith|endsWith|test|match|split|replace|replaceAll|join)$|cn$|clsx$|cva$|t$|rich$)/.test(p.expression.getText(sf)))
      if (!skip && text !== null && (proseString(text) || (endsInValuePosition(n) && tableValue(text))) && !(MODULE && ts.isTemplateExpression(n))) replaceLiteral(n)
      return
    }
    // a JSX expression child with string leaves, when it is not part of a bigger unit: handled by the unit below, or here alone
    if ((ts.isJsxElement(n) || ts.isJsxFragment(n)) && inFunction(n)) {
      const kids = n.children
      const textBearing = kids.some((k) => ts.isJsxText(k) && hasText(cleanJsxText(k.text)))
      if (textBearing) {
        const used = new Set()
        const segs = segmentsOf(kids, used)
        if (!segs) return
        const params = []
        const tags = []
        const msg = render(segs, params, tags)
        const id = idFor(msg, params)
        const first = kids[0]
        const last = kids[kids.length - 1]
        let call
        if (tags.length || params.some(([nm]) => msg.includes(`{${nm}, node}`))) {
          usesRich = true
          call = `{rich('${id}', ${paramsObject(params, tags)})}`
        } else {
          usesT = true
          call = `{${T}('${id}'${params.length ? `, ${paramsObject(params, [])}` : ''})}`
        }
        // leading/trailing whitespace-only text stays out of the range: replace from first to last child
        take(first.getStart(sf, false), last.end)
        edits.push({ s: first.pos, e: last.end, text: call })
        return
      }
      // no words of its own: string leaves of expression children
      for (const k of kids) {
        if (ts.isJsxExpression(k) && k.expression) {
          leaves(k.expression, (l) => {
            const text = litOf(l)
            if (hasText(text) && !edits.some((x) => x.s === l.getStart(sf))) replaceLiteral(l)
          })
        }
      }
    }
  }
  visit(sf)

  if (edits.length === 0) {
    console.log(`${file}: nothing to do`)
    continue
  }
  edits.sort((a, b) => a.s - b.s)
  // drop edits nested in a bigger one (kept: the outer)
  const flat = []
  for (const e of edits) {
    const last = flat[flat.length - 1]
    if (last && e.s < last.e) continue
    flat.push(e)
  }
  let out = ''
  let pos = 0
  for (const e of flat) {
    out += src.slice(pos, e.s) + e.text
    pos = e.e
  }
  out += src.slice(pos)
  // imports
  const imports = []
  if ((usesT || usesMsg) && !/from '@\/i18n'/.test(out)) imports.push(`import { ${[usesT ? `t${usesTName ? ' as tt' : ''}` : '', usesMsg ? 'msg' : ''].filter(Boolean).join(', ')} } from '@/i18n'`)
  else if (usesMsg && /import \{([^}]*)\} from '@\/i18n'/.test(out) && !/import \{[^}]*\bmsg\b[^}]*\} from '@\/i18n'/.test(out)) out = out.replace(/import \{([^}]*)\} from '@\/i18n'/, (m0, g) => `import {${g.replace(/\s+$/, '')}, msg } from '@/i18n'`)
  if (usesRich && !/from '@\/i18n\/react'/.test(out)) imports.push(`import { rich } from '@/i18n/react'`)
  if (!out.includes(`'@/i18n/en/${area}'`) && (usesT || usesRich || usesMsg)) imports.push(`import '@/i18n/en/${area}'`)
  if (imports.length) {
    const m = [...out.matchAll(/^import [^\n]*\n/gm)]
    const lastImport = m.length ? m[m.length - 1] : null
    const at = lastImport ? lastImport.index + lastImport[0].length : 0
    out = out.slice(0, at) + imports.join('\n') + '\n' + out.slice(at)
  }
  writeFileSync(file, out)
  console.log(`${file}: ${flat.length} edits`)
}

// write the catalogue
if (added.length) {
  let text = cat.text
  const marker = '} as const'
  const at = text.lastIndexOf(marker)
  const lines = added.map(([id, q]) => `  '${id}': ${q},\n`).join('')
  text = text.slice(0, at) + lines + text.slice(at)
  writeFileSync(catFile, text)
  console.log(`${catFile}: +${added.length} messages`)
}
