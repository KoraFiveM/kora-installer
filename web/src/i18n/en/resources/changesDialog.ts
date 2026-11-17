import { register } from '../../index'

const messages = {
  'resources.changesDialog.throwChangesAway': 'Throw your changes away?',
  'resources.changesDialog.changesSaved': 'You have changes that are not saved',
  'resources.changesDialog.thisResource': 'This resource',
  'resources.changesDialog.countResourcesChangesOwn': '{count} resources have changes of your own: {join}.',
  'resources.changesDialog.valueChangesOwnWere': '{value} has changes of your own that were never saved as a commit.',
  'resources.changesDialog.changedFiles': 'Changed files',
  'resources.changesDialog.cannotUndone': 'This cannot be undone',
  'resources.changesDialog.changedFilesGoBack': 'The changed files go back to how they were, and any new files you added in {value} are deleted. Kora keeps no copy.',
  'resources.changesDialog.goBack': 'Go back',
  'resources.changesDialog.throwMyChangesAway': 'Throw my changes away',
  'resources.changesDialog.koraSwitchUntilChoose': 'Kora will not switch until you choose, so nothing is lost by accident.',
  'resources.changesDialog.keepMyChangesAside': 'Keep my changes aside',
  'resources.changesDialog.recommendedTheySetAside': 'Recommended. They are set aside safely and you can bring them back later from this panel.',
  'resources.changesDialog.throwMyChangesAway2': 'Throw my changes away…',
  'resources.changesDialog.askedConfirm': 'You will be asked to confirm.',
  'resources.changesDialog.cancelChangeNothing': 'Cancel, change nothing',
} as const

register('en', messages)
export default messages
