import { chromium } from '@playwright/test'
const [path, width = '320'] = process.argv.slice(2)
const b = await chromium.launch()
const p = await b.newPage({ viewport: { width: Number(width), height: 800 } })
await p.goto(`http://127.0.0.1:4391${path}${path.includes('?') ? '&' : '?'}scenario=default&as=local`)
await p.waitForTimeout(1500)
console.log("sw", await p.evaluate(() => document.documentElement.scrollWidth + " " + document.documentElement.clientWidth))
console.log(await p.evaluate(() => {
  const vw = document.documentElement.clientWidth
  const out = []
  for (const e of document.querySelectorAll('body *')) { const r = e.getBoundingClientRect(); if (r.right > vw + 1 && r.width > 0) out.push(`${e.tagName.toLowerCase()}.${String(e.className).slice(0, 70)} right=${Math.round(r.right)} w=${Math.round(r.width)}`) }
  return out.slice(0, 14).join('\n')
}))
await b.close()
