import { register } from '../../index'

const messages = {
  'core.errorBoundary.pageNeedsReloaded': 'This page needs to be reloaded',
  'core.errorBoundary.koraProbablyUpdatedWhile': 'Kora was probably updated while this page was open, so the part of the page you asked for is not there any more. Your servers are not affected. Reload the page to get the new version.',
  'core.errorBoundary.reloadPage': 'Reload the page',
  'core.errorBoundary.somethingWentWrong': 'Something went wrong',
  'core.errorBoundary.screenHitProblemCould': 'This screen hit a problem and could not be shown. Your servers are not affected.',
  'core.errorBoundary.tryAgain': 'Try again',
  'core.errorBoundary.backServers': 'Back to your servers',
} as const

register('en', messages)
export default messages
