import { register } from '../../index'

const messages = {
  'core.utils.portsMustWholeNumbers': 'Ports must be whole numbers from 1024 to 65535.',
  'core.utils.gamePortTxadminPort': 'The game port and the txAdmin port must be different.',
} as const

register('en', messages)
export default messages
