import { describe, expect, it } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import { ansiSegments, parseAnsi } from './ansi'

const html = (s: string) => renderToStaticMarkup(<>{parseAnsi(s)}</>)

describe('ansiSegments', () => {
  it('returns plain text unchanged', () => {
    expect(ansiSegments('Server started on port 30120')).toEqual([{ text: 'Server started on port 30120', style: {} }])
  })

  it('applies a 16-colour foreground and resets with 39 and 0', () => {
    const segs = ansiSegments('\x1b[31mred\x1b[39m plain \x1b[92mgreen\x1b[0m end')
    expect(segs.map((s) => s.text)).toEqual(['red', ' plain ', 'green', ' end'])
    expect(segs[0].style.fg).toBe('#ef4444')
    expect(segs[1].style.fg).toBeUndefined()
    expect(segs[2].style.fg).toBe('#4ade80')
    expect(segs[3].style).toEqual({})
  })

  it('handles bold, dim, italic and underline and their resets', () => {
    const segs = ansiSegments('\x1b[1;2;3;4mall\x1b[22mnobold\x1b[23;24m none')
    expect(segs[0].style).toEqual({ bold: true, dim: true, italic: true, underline: true })
    expect(segs[1].style).toEqual({ italic: true, underline: true })
    expect(segs[2].style).toEqual({})
  })

  it('sets backgrounds for 40-47, 100-107 and resets with 49', () => {
    const segs = ansiSegments('\x1b[44mbg\x1b[101mbright\x1b[49mend')
    expect(segs[0].style.bg).toBe('#4aa8ff')
    expect(segs[1].style.bg).toBe('#f87171')
    expect(segs[2].style.bg).toBeUndefined()
  })

  it('supports 256-colour indexes for both foreground and background', () => {
    const segs = ansiSegments('\x1b[38;5;196mA\x1b[48;5;21mB\x1b[38;5;240mC')
    expect(segs[0].style.fg).toBe('#ff0000') // cube: 196 = 5,0,0
    expect(segs[1].style.bg).toBe('#0000ff') // cube: 21 = 0,0,5
    expect(segs[2].style.fg).toBe('#585858') // grey ramp: 240
  })

  it('supports true colour', () => {
    const segs = ansiSegments('\x1b[38;2;10;255;128mX\x1b[48;2;300;0;5mY')
    expect(segs[0].style.fg).toBe('#0aff80')
    expect(segs[1].style.bg).toBe('#ff0005') // out-of-range values are clamped to 0-255
    expect(ansiSegments('\x1b[48;2;1;-5;0mZ')[0].style).toEqual({}) // non-digit parameters: sequence ignored
  })

  it('ignores malformed or unterminated extended colours', () => {
    expect(ansiSegments('\x1b[38;5mtext')[0]).toEqual({ text: 'text', style: {} })
    expect(ansiSegments('\x1b[38;2;1;2mtext')[0].style).toEqual({})
  })

  it('ignores unsupported SGR codes but keeps the text', () => {
    const segs = ansiSegments('\x1b[7;5;53mtext')
    expect(segs).toEqual([{ text: 'text', style: {} }])
  })

  it('removes non-SGR escape sequences such as cursor moves and OSC titles', () => {
    expect(ansiSegments('a\x1b[2Kb\x1b[1Gc')).toEqual([{ text: 'abc', style: {} }])
    expect(ansiSegments('\x1b]0;window title\x07done')).toEqual([{ text: 'done', style: {} }])
    expect(ansiSegments('\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\')).toEqual([{ text: 'link', style: {} }])
  })

  it('drops malformed and unterminated sequences without losing the text before them', () => {
    expect(ansiSegments('before\x1b[31')).toEqual([{ text: 'before', style: {} }])
    expect(ansiSegments('before\x1b')).toEqual([{ text: 'before', style: {} }])
    expect(ansiSegments('before\x1b]0;never ends')).toEqual([{ text: 'before', style: {} }])
    expect(ansiSegments('\x1b[31mred then \x1b[')).toEqual([{ text: 'red then ', style: { fg: '#ef4444' } }])
  })

  it('strips control characters but keeps tabs and newlines', () => {
    expect(ansiSegments('a\x00b\x07c\x08d\re\tf')).toEqual([{ text: 'abcde\tf', style: {} }])
  })

  it('resets colour at each newline', () => {
    const segs = ansiSegments('\x1b[31mone\ntwo')
    expect(segs.map((s) => s.text).join('')).toBe('one\ntwo')
    expect(segs[segs.length - 1].style).toEqual({})
  })
})

describe('parseAnsi rendering', () => {
  it('renders coloured text as spans with inline styles', () => {
    expect(html('\x1b[31mred\x1b[0m')).toBe('<span style="color:#ef4444">red</span>')
  })

  it('renders plain text without any wrapper', () => {
    expect(html('plain text')).toBe('plain text')
  })

  it('keeps script-like text as text', () => {
    const out = html('\x1b[33m<script>alert(1)</script><img src=x onerror=alert(1)>')
    expect(out).not.toContain('<script')
    expect(out).not.toContain('<img')
    expect(out).toContain('&lt;script&gt;alert(1)&lt;/script&gt;')
  })

  it('does not interpret HTML-like colour text as styling', () => {
    expect(html('<span style="color:red">x</span>')).toBe('&lt;span style=&quot;color:red&quot;&gt;x&lt;/span&gt;')
  })
})
