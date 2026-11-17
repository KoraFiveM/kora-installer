import { register } from '../../index'

const messages = {
  'core.notFound.pageFound': 'Page not found',
  'core.notFound.thereNothingAddressMay': 'There is nothing at this address. It may have been removed or typed wrongly.',
  'core.notFound.backServers': 'Back to your servers',
} as const

register('en', messages)
export default messages
