import { chromium } from '@playwright/test'
const [path, width = '320'] = process.argv.slice(2)
const b = await chromium.launch()
const p = await b.newPage({ viewport: { width: Number(width), height: 800 } })
await p.goto(`http://127.0.0.1:4391${path}${path.includes('?') ? '&' : '?'}scenario=default&as=local`)
await p.waitForTimeout(1500)
console.log(await p.evaluate(() => {
  const root = document.documentElement
  const base = root.scrollWidth
  const out = []
  const walk = (el, depth) => {
    for (const c of el.children) {
      const prev = c.style.display
      c.style.display = 'none'
      const w = root.scrollWidth
      c.style.display = prev
      if (w < base) { out.push(`${'  '.repeat(depth)}${c.tagName.toLowerCase()}.${String(c.className).slice(0, 60)} ${base}->${w}`); if (depth < 8) walk(c, depth + 1) }
    }
  }
  walk(document.body, 0)
  return out.join('\n')
}))
await b.close()
