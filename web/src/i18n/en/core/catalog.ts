import { register } from '../../index'

const messages = {
  'core.catalog.requiredFirst': 'Required first',
  'core.catalog.core': 'Core',
  'core.catalog.standalone': 'Standalone',
  'core.catalog.cfxReDefaults': 'Cfx.re defaults',
  'core.catalog.defaultMaps': 'Default maps',
  'core.catalog.compatibility': 'Compatibility',
  'core.catalog.props': 'Props',
  'core.catalog.bridges': 'Bridges',
  'core.catalog.creators': 'Creators',
  'core.catalog.other': 'Other',
} as const

register('en', messages)
export default messages
