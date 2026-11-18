import { register } from '../../index'

const messages = {
  'resources.copyPanel.copyOriginal': ', a copy of {original}',
  'resources.copyPanel.ownCopyNodeNode': 'Your own copy: {a, node}{node, node}.',
  'resources.copyPanel.ownCopy': 'Your own copy',
  'resources.copyPanel.theOriginal': 'the original',
  'resources.copyPanel.resourcePlainCopyStrong': 'This resource is a plain copy of {strong, node}, so you cannot send changes anywhere or open a pull request from it. Making your own copy (a fork on GitHub) keeps what you have: the same version, the same branch and any changes you made.',
  'resources.copyPanel.nodeNodeMakeMy': '{node, node}Make my own copy (fork)',
  'resources.copyPanel.doWholeServer': 'Do it for the whole server',
  'resources.copyPanel.someCouldCopied': 'Some could not be copied',
  'resources.copyPanel.done': 'Done',
  'resources.copyPanel.serverNowUsesOwn': 'This server now uses your own copies.',
} as const

register('en', messages)
export default messages
