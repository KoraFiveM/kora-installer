import { register } from '../../index'

const messages = {
  'core.useRole.accountRoleCannotDo': 'Your account ({role}) cannot do this. Ask an administrator for {min} access.',
  'core.useRole.turnedOffSigninsDo': 'This is turned off for sign-ins that do not come through Tailscale, because a sign-in that can be copied (a token, a stolen browser session) could leak. To turn it on, run this on the computer that runs Kora (over SSH is fine): {ALLOW_REMOTE_COMMAND}. Or sign in through Tailscale as an administrator, or use the panel on that computer.',
} as const

register('en', messages)
export default messages
