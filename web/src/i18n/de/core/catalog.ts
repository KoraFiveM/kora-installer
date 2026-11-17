import { register } from '../../index'
import type en from '../../en/core/catalog'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.catalog.requiredFirst': 'Zuerst nötig',
  'core.catalog.core': 'Kern',
  'core.catalog.standalone': 'Eigenständig',
  'core.catalog.cfxReDefaults': 'Cfx.re-Standards',
  'core.catalog.defaultMaps': 'Standard-Maps',
  'core.catalog.compatibility': 'Kompatibilität',
  'core.catalog.props': 'Props',
  'core.catalog.bridges': 'Brücken',
  'core.catalog.creators': 'Creator',
  'core.catalog.other': 'Sonstiges',
} satisfies Record<keyof typeof en, string>

register('de', messages)
