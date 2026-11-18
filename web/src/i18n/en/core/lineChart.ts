import { register } from '../../index'

const messages = {
  'core.lineChart.titleNowFormatHighest': '{title}: now {format}, highest {format2}.',
  'core.lineChart.titleNoMeasurementsYet': '{title}: no measurements yet.',
  'core.lineChart.nowFormatHighestFormat': 'now {format} · highest {format2}',
  'core.lineChart.noData': 'no data',
  'core.lineChart.topScaleFormat': 'top of scale {format}',
} as const

register('en', messages)
export default messages
