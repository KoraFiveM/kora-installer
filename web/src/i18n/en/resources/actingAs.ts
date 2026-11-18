import { register } from '../../index'

const messages = {
  'resources.actingAs.doneGithubStrongNode': 'This is done on GitHub as {strong, node}, the account this computer is signed in as. It is shared by everyone who uses this panel.',
  'resources.actingAs.doneGithubAccountComputer': 'This is done as the GitHub account this computer is signed in as, shared by everyone who uses this panel.',
} as const

register('en', messages)
export default messages
