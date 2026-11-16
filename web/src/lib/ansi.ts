import { createElement, type CSSProperties, type ReactNode } from 'react'

/*
 * A small, safe ANSI (terminal) colour parser for server console and log output.
 *
 * - Understands SGR (ESC[...m) codes: reset, bold, dim, italic, underline and their resets,
 *   16 colours, 256 colours (38;5;n / 48;5;n) and true colour (38;2;r;g;b / 48;2;r;g;b).
 * - Every other escape sequence (cursor moves, OSC titles, etc.) is removed, as are control characters.
 * - Output is React elements and text nodes only. Nothing is ever injected as HTML, so text that
 *   looks like markup (for example <script>) is shown as plain text.
 * - Colour state is reset at each newline, so one broken sequence cannot colour the lines after it.
 */

export type AnsiStyle = {
  fg?: string
  bg?: string
  bold?: boolean
  dim?: boolean
  italic?: boolean
  underline?: boolean
}

export type AnsiSegment = { text: string; style: AnsiStyle }

// Standard and bright colours, tuned for the panel's dark theme (ink #0f0f0f, card #151515).
const BASE = ['#4b4b4b', '#ef4444', '#22c55e', '#eab308', '#4aa8ff', '#c084fc', '#22d3ee', '#d4d4d4']
const BRIGHT = ['#8e8e8e', '#f87171', '#4ade80', '#facc15', '#7cc0ff', '#e9a8ff', '#67e8f9', '#ffffff']

const CUBE = [0, 95, 135, 175, 215, 255]

function hex(n: number): string {
  return '#' + n.toString(16).padStart(2, '0')
}

/** Colour for a 256-colour palette index (0-255). */
function paletteColour(n: number): string {
  if (n < 8) return BASE[n]
  if (n < 16) return BRIGHT[n - 8]
  if (n < 232) {
    const i = n - 16
    return '#' + [Math.floor(i / 36), Math.floor((i % 36) / 6), i % 6].map((c) => hex(CUBE[c]).slice(1)).join('')
  }
  const g = 8 + (n - 232) * 10
  return '#' + hex(g).slice(1).repeat(3)
}

const clamp = (n: number) => Math.min(255, Math.max(0, n))

/** Apply the parameters of one SGR sequence (e.g. "1;31") to a style. Unknown codes are ignored. */
function applySgr(style: AnsiStyle, params: string): AnsiStyle {
  const codes = params === '' ? [0] : params.split(';').map((p) => (p === '' ? 0 : Number(p)))
  let s = { ...style }
  for (let i = 0; i < codes.length; i++) {
    const c = codes[i]
    if (!Number.isFinite(c)) continue
    if (c === 0) s = {}
    else if (c === 1) s.bold = true
    else if (c === 2) s.dim = true
    else if (c === 3) s.italic = true
    else if (c === 4) s.underline = true
    else if (c === 22) { delete s.bold; delete s.dim }
    else if (c === 23) delete s.italic
    else if (c === 24) delete s.underline
    else if (c >= 30 && c <= 37) s.fg = BASE[c - 30]
    else if (c === 39) delete s.fg
    else if (c >= 40 && c <= 47) s.bg = BASE[c - 40]
    else if (c === 49) delete s.bg
    else if (c >= 90 && c <= 97) s.fg = BRIGHT[c - 90]
    else if (c >= 100 && c <= 107) s.bg = BRIGHT[c - 100]
    else if (c === 38 || c === 48) {
      const key = c === 38 ? 'fg' : 'bg'
      if (codes[i + 1] === 5 && Number.isFinite(codes[i + 2])) {
        s[key] = paletteColour(clamp(codes[i + 2]))
        i += 2
      } else if (codes[i + 1] === 2 && Number.isFinite(codes[i + 4])) {
        s[key] = '#' + [codes[i + 2], codes[i + 3], codes[i + 4]].map((v) => hex(clamp(v)).slice(1)).join('')
        i += 4
      } else {
        break // malformed extended colour: ignore the rest of this sequence
      }
    }
  }
  return s
}

function sameStyle(a: AnsiStyle, b: AnsiStyle): boolean {
  return a.fg === b.fg && a.bg === b.bg && !!a.bold === !!b.bold && !!a.dim === !!b.dim && !!a.italic === !!b.italic && !!a.underline === !!b.underline
}

const isDigitOrSep = (c: string) => c >= '0' && c <= '9' || c === ';'

/** Split text into styled segments. Escape sequences and control characters never appear in the text. */
export function ansiSegments(input: string): AnsiSegment[] {
  const out: AnsiSegment[] = []
  let style: AnsiStyle = {}
  let buf = ''
  const flush = () => {
    if (buf) {
      const last = out[out.length - 1]
      if (last && sameStyle(last.style, style)) last.text += buf
      else out.push({ text: buf, style })
      buf = ''
    }
  }
  const s = input
  const n = s.length
  let i = 0
  while (i < n) {
    const ch = s[i]
    if (ch === '\x1b' || ch === '\x9b') {
      // CSI: ESC [ params intermediates final, or the single-byte form \x9b.
      const csi = ch === '\x9b' || s[i + 1] === '['
      if (csi) {
        let k = ch === '\x9b' ? i + 1 : i + 2
        const pStart = k
        while (k < n && s.charCodeAt(k) >= 0x30 && s.charCodeAt(k) <= 0x3f) k++
        const params = s.slice(pStart, k)
        const iStart = k
        while (k < n && s.charCodeAt(k) >= 0x20 && s.charCodeAt(k) <= 0x2f) k++
        if (k >= n) break // unterminated: drop the rest
        const final = s[k]
        const code = s.charCodeAt(k)
        if (code >= 0x40 && code <= 0x7e) {
          if (final === 'm' && k === iStart && params.split('').every(isDigitOrSep)) {
            flush()
            style = applySgr(style, params)
          }
          i = k + 1
          continue
        }
        // Not a valid final byte: drop the partial sequence up to here.
        i = k
        continue
      }
      // OSC (ESC ]) and string sequences (ESC P, X, ^, _): drop up to BEL or ST (ESC \).
      const next = s[i + 1]
      if (next === ']' || next === 'P' || next === 'X' || next === '^' || next === '_') {
        let k = i + 2
        while (k < n && s[k] !== '\x07' && !(s[k] === '\x1b' && s[k + 1] === '\\') && s[k] !== '\x9c') k++
        if (k >= n) break // unterminated OSC/string: drop the rest
        i = s[k] === '\x1b' ? k + 2 : k + 1
        continue
      }
      // Other escape sequences: ESC, optional intermediates, one final byte.
      let k = i + 1
      while (k < n && s.charCodeAt(k) >= 0x20 && s.charCodeAt(k) <= 0x2f) k++
      i = k + 1
      continue
    }
    if (ch === '\n' || ch === '\t') {
      buf += ch
      if (ch === '\n') { flush(); style = {} } // each line starts with the default colour
      i++
      continue
    }
    const code = ch.charCodeAt(0)
    // Drop C0 controls (including \r), DEL and C1 controls.
    if (code < 0x20 || (code >= 0x7f && code <= 0x9f)) { i++; continue }
    buf += ch
    i++
  }
  flush()
  return out
}

/** Convert a style to inline CSS. Empty for the default style. */
export function styleToCss(style: AnsiStyle): CSSProperties {
  const css: CSSProperties = {}
  if (style.fg) css.color = style.fg
  if (style.bg) css.backgroundColor = style.bg
  if (style.bold) css.fontWeight = 700
  if (style.dim) css.opacity = 0.6
  if (style.italic) css.fontStyle = 'italic'
  if (style.underline) css.textDecoration = 'underline'
  return css
}

/** React nodes for ANSI text: plain strings where there is no styling, <span style> elements otherwise. */
export function parseAnsi(input: string): ReactNode[] {
  return ansiSegments(input).map((seg, i) => {
    const css = styleToCss(seg.style)
    return Object.keys(css).length === 0 ? seg.text : createElement('span', { key: i, style: css }, seg.text)
  })
}
