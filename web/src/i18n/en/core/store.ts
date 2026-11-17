import { register } from '../../index'

const messages = {
  'core.store.nSignedFromAnother': '{n} (signed in from another device{kind, select, both { with {provider}, {role}} provider { with {provider}} role {, {role}} other {}})',
  'core.store.nKoraCommand': '{n} (a kora command)',
  'core.store.nComputer': '{n} (on this computer)',
  'core.store.someone': 'someone',
} as const

register('en', messages)
export default messages
