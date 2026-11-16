// The floating "Mock" control (bottom-right): change scenario, reset state, open the styleguide.
// Only bundled in mock builds. It has its own React root, so it stays available on the "session expired" screen.
import { useId, useState } from 'react'
import { SCENARIOS, SCENARIO_INFO, switchScenario, type ScenarioName } from './scenarios'

export function MockSwitcher({ scenario }: { scenario: ScenarioName }) {
  const [open, setOpen] = useState(false)
  const [picked, setPicked] = useState<ScenarioName>(scenario)
  const panelId = useId()
  const selectId = useId()
  const box = 'fixed bottom-3 right-3 z-[100] text-xs text-white'
  return (
    <div className={box}>
      {open && (
        <div id={panelId} role="group" aria-label="Mock backend controls" className="mb-2 w-72 space-y-3 rounded-lg border border-amber/60 bg-card p-3 shadow-xl">
          <p className="font-semibold text-amber">Demo data: nothing here is real.</p>
          <div className="space-y-1">
            <label htmlFor={selectId} className="block font-medium">Scenario</label>
            <select id={selectId} value={picked} onChange={(e) => { const v = e.target.value as ScenarioName; setPicked(v); switchScenario(v) }} className="h-8 w-full rounded-md border border-input bg-ink px-2">
              {SCENARIOS.map((n) => <option key={n} value={n}>{n}</option>)}
            </select>
            <p className="text-muted-foreground">{SCENARIO_INFO[picked]}</p>
          </div>
          <div className="flex gap-2">
            <button type="button" className="h-8 rounded-md border border-border bg-secondary px-3 font-medium hover:bg-secondary/80" onClick={() => switchScenario(scenario)}>Reset state</button>
            <a className="inline-flex h-8 items-center rounded-md border border-border bg-secondary px-3 font-medium hover:bg-secondary/80" href="/styleguide">Styleguide</a>
          </div>
        </div>
      )}
      <div className="flex justify-end">
        <button type="button" aria-expanded={open} aria-controls={open ? panelId : undefined} onClick={() => setOpen(!open)} className="rounded-full border border-amber/60 bg-card px-3 py-1.5 font-medium shadow-lg hover:bg-secondary">
          <span aria-hidden className="mr-1.5 inline-block size-1.5 rounded-full bg-amber align-middle" />
          Mock: {scenario}
        </button>
      </div>
    </div>
  )
}
