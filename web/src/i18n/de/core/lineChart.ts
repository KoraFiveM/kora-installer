import { register } from '../../index'
import type en from '../../en/core/lineChart'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.lineChart.titleNowFormatHighest': '{title}: jetzt {format}, Höchstwert {format2}.',
  'core.lineChart.titleNoMeasurementsYet': '{title}: noch keine Messwerte.',
  'core.lineChart.nowFormatHighestFormat': 'jetzt {format} · Höchstwert {format2}',
  'core.lineChart.noData': 'keine Daten',
  'core.lineChart.topScaleFormat': 'Skalenende {format}',
} satisfies Record<keyof typeof en, string>

register('de', messages)
