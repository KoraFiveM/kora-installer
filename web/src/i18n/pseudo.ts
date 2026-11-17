/**
 * The pseudo-language: English with accents and more letters. (No brackets: a visible label and its aria-label must still start with the same text, WCAG 2.5.3.) Anything that still reads as plain
 * English on screen was never put in the catalogue; anything that breaks was too tight for a long language.
 */
const MAP: Record<string, string> = {
  a: 'á', b: 'ƀ', c: 'ç', d: 'ď', e: 'é', f: 'ƒ', g: 'ĝ', h: 'ĥ', i: 'í', j: 'ĵ', k: 'ķ', l: 'ĺ', m: 'ɱ', n: 'ñ', o: 'ó', p: 'þ', q: 'ɋ', r: 'ŕ', s: 'š', t: 'ţ', u: 'ú', v: 'ṽ', w: 'ŵ', x: 'ẋ', y: 'ý', z: 'ž',
  A: 'Á', B: 'Ɓ', C: 'Ç', D: 'Ď', E: 'É', F: 'Ƒ', G: 'Ĝ', H: 'Ĥ', I: 'Í', J: 'Ĵ', K: 'Ķ', L: 'Ĺ', M: 'Ṁ', N: 'Ñ', O: 'Ó', P: 'Þ', Q: 'Ɋ', R: 'Ŕ', S: 'Š', T: 'Ţ', U: 'Ú', V: 'Ṽ', W: 'Ŵ', X: 'Ẋ', Y: 'Ý', Z: 'Ž',
}
const VOWELS = /[aeiouAEIOU]/

export function pseudo(text: string): string {
  if (!/[A-Za-z]/.test(text)) return text
  let out = ''
  let vowels = 0
  for (const c of text) {
    out += MAP[c] ?? c
    // Double every second vowel (counted from the start of the text, so a label and the longer label it begins
    // are drawn the same way): "Servers" gets longer without becoming unreadable.
    if (VOWELS.test(c) && ++vowels % 2 === 0) out += MAP[c] ?? c
  }
  return out
}
