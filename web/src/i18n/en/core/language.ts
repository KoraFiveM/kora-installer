import { register } from '../../index'

const messages = {
  'core.language.label': 'Language',
  'core.language.beta': '{name} (beta)',
  'core.language.hint': 'Beta: machine-assisted and not yet checked by a native speaker. Texts that are not translated yet show in English.',
  'core.language.remembered': 'The choice is remembered by this browser. Some texts (the command line, logs and some messages from the server) are only in English.',
} as const

register('en', messages)
export default messages
