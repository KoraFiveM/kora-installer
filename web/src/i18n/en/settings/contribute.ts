import { register } from '../../index'

const messages = {
  'settings.contribute.title': 'Contribute to Kora',
  'settings.contribute.intro': 'Found a bug, or want to change something? You can suggest it as a pull request (a PR) on the Kora repository. You need a GitHub account, which you already have if you signed in above.',
  'settings.contribute.step1': 'Open the Kora repository on GitHub and press Fork. This makes your own copy.',
  'settings.contribute.step2': 'In your fork, make the change on a new branch.',
  'settings.contribute.step3': 'On your fork, press Contribute, then Open pull request, and send it to the main branch of the Kora repository.',
  'settings.contribute.openRepo': 'Open the Kora repository',
  'settings.contribute.guide': 'Read the contributing guide',
} as const

register('en', messages)
export default messages
