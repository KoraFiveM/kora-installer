import { register } from '../../index'
import type en from '../../en/auth/signedOut'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'auth.signedOut.signIn': 'Anmelden',
  'auth.signedOut.oldTokenLink': 'Alter Token-Link',
  'auth.signedOut.signinLink': 'Anmelde-Link',
  'auth.signedOut.onlyStillTokenFrom': 'Nur, falls du noch einen Token von früher hast. Er funktioniert vorerst noch, und das Panel hilft dir, ihn durch einen Passkey zu ersetzen.',
  'auth.signedOut.pasteWholeLinkCommand': 'Füge den ganzen Link ein, den der Befehl ausgegeben hat. Er beginnt mit http://.',
  'auth.signedOut.signIn2': 'Anmelden ',
  'auth.signedOut.signKora': 'Bei Kora anmelden',
  'auth.signedOut.signPasskeySetUp': 'Melde dich mit dem eingerichteten Passkey oder mit einem Einmalcode an. Neu hier? Öffne den Einmal-Link, den dir die Person gegeben hat, die Kora betreibt. ',
  'auth.signedOut.signTailscale': 'Mit Tailscale anmelden',
  'auth.signedOut.administratorRequiresPasskeyOn': 'Ein Administrator verlangt hier einen Passkey oder Einmalcodes, deshalb wird der alte Token-Link nicht akzeptiert.',
  'auth.signedOut.iOldTokenLink': 'Ich habe einen alten Token-Link',
  'auth.signedOut.keepServersSafePage': 'Damit deine Server sicher bleiben, braucht diese Seite einen Anmelde-Link. Führe <code>kora panel --print-link</code> auf diesem Computer aus, um einen neuen Anmelde-Link zu erhalten. Öffne den Link oder füge ihn unten ein. Wenn der Befehl sagt, dass der Hintergrunddienst nicht installiert ist, führe stattdessen <code2>kora panel</code2> aus und benutze den Link, den es ausgibt. ',
} satisfies Record<keyof typeof en, string>

register('de', messages)
