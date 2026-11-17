import { register } from '../index'

const messages = {
  'system.systemCheckPage.systemCheck': 'System check',
  'system.systemCheckPage.whatKoraNeedsComputer': 'What Kora needs on this computer, and a button to fix anything that is missing. Most fixes are a button; where one needs a command, it is shown to copy.',
  'system.systemCheckPage.nodeNodeCheckAgain': '{node, node}Check again ',
  'system.systemCheckPage.loaderNodeCheckingComputer': '{loader2, node}Checking this computer...',
  'system.systemCheckPage.everythingKoraNeedsHere': 'Everything Kora needs is here.',
  'system.systemCheckPage.thingNeedsFixing': '1 thing needs fixing.',
  'system.systemCheckPage.problemsThingsNeedFixing': '{problems} things need fixing.',
  'system.systemCheckPage.accountRoleLookBut': 'Your account ({role}) can look at this but not fix it. Ask an administrator to open this page and use the buttons, or to run the commands shown.',
  'system.systemCheckPage.koraRunsCodeNode': 'Kora runs {code, node}',
} as const

register('en', messages)
export default messages
