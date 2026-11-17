// Scratch probe (not committed): node e2e/probe.mjs <path> [width]
import { chromium } from '@playwright/test'
const [path, width = '1280'] = process.argv.slice(2)
const b = await chromium.launch()
const p = await b.newPage({ viewport: { width: Number(width), height: 800 } })
await p.goto(`http://127.0.0.1:4391${path}${path.includes('?') ? '&' : '?'}scenario=default&as=local`)
await p.waitForTimeout(1500)
console.log('BUTTONS', JSON.stringify(await p.getByRole('button').evaluateAll((els) => els.map((e) => (e.getAttribute('aria-label') || e.textContent || '').trim().slice(0, 40)))))
if (process.env.OVERFLOW) {
  console.log(await p.evaluate(() => {
    const vw = document.documentElement.clientWidth
    const out = []
    for (const e of document.querySelectorAll('body *')) {
      const r = e.getBoundingClientRect()
      let a = e.parentElement, scrolled = false
      while (a) { const o = getComputedStyle(a).overflowX; if (o === 'auto' || o === 'scroll' || o === 'hidden') { scrolled = true; break } a = a.parentElement }
      if (!scrolled && r.right > vw + 1 && r.width > 0) out.push(`${e.tagName.toLowerCase()}.${String(e.className).slice(0, 60)} right=${Math.round(r.right)} w=${Math.round(r.width)}`)
    }
    return out.slice(0, 12).join('\n')
  }))
}
await b.close()
