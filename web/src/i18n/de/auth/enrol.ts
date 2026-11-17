import { register } from '../../index'
import type en from '../../en/auth/enrol'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'auth.enrol.setUpSignin': 'Richte deine Anmeldung ein',
  'auth.enrol.panelDidStartSetup': 'Das Panel hat die Einrichtung nicht gestartet.',
  'auth.enrol.myPasskey': 'Mein Passkey',
  'auth.enrol.welcomeUser': 'Willkommen, {user}',
  'auth.enrol.checkingLink': 'Dein Link wird geprüft…',
  'auth.enrol.wereInvitedKoraPanel': 'Du wurdest als {strong, node} zu Kora Panel eingeladen. Dieser Link funktioniert nur einmal. Richte ein, wie du dich ab jetzt anmeldest, damit niemand mehr einen geheimen Link weitergeben muss. ',
  'auth.enrol.passkeyRecommended': 'Passkey (empfohlen)',
  'auth.enrol.phoneLaptopSecurityKey': 'Dein Handy, Laptop oder Sicherheitsschlüssel verwahrt einen privaten Schlüssel und fragt nach Fingerabdruck, Gesicht oder PIN. Nichts zu merken oder einzutippen.',
  'auth.enrol.nameDevice': 'Gib diesem Gerät einen Namen',
  'auth.enrol.tellPasskeysApartLater': 'Damit du deine Passkeys später unterscheiden kannst, zum Beispiel Arbeitslaptop.',
  'auth.enrol.myLaptop': 'Mein Laptop',
  'auth.enrol.setUpPasskey': 'Passkey einrichten',
  'auth.enrol.browserCannotMakePasskey': 'Dieser Browser kann hier keinen Passkey erstellen.',
  'auth.enrol.valueUsePasswordOnetime': '{value} Du kannst stattdessen ein Passwort und Einmalcodes benutzen.',
  'auth.enrol.passwordOnetimeCodes': 'Passwort und Einmalcodes',
  'auth.enrol.worksEverywhereNeedAuthenticat': 'Funktioniert überall. Du brauchst eine Authenticator-App (jede App, die einen QR-Code scannt: die deines Handys, 1Password, Aegis und so weiter).',
  'auth.enrol.setUpOnetimeCodes': 'Einmalcodes einrichten',
  'auth.enrol.qrCodeAuthenticatorApp': 'QR-Code für deine Authenticator-App',
  'auth.enrol.setupKey': 'Einrichtungsschlüssel',
  'auth.enrol.scanAuthenticatorAppImg': 'Scanne das mit deiner Authenticator-App. {img, node}<p>Kannst du nicht scannen? Gib stattdessen diesen Schlüssel in die App ein:</p>{copyField, node}',
  'auth.enrol.choosePassword': 'Wähle ein Passwort',
  'auth.enrol.leastCharactersFewUnrelated': 'Mindestens 12 Zeichen. Ein paar zusammenhanglose Wörter ergeben ein starkes, leicht zu merkendes Passwort.',
  'auth.enrol.enterDigitCodeApp': 'Gib den 6-stelligen Code ein, den die App anzeigt',
  'auth.enrol.finishSetup': 'Einrichtung abschließen',
  'auth.enrol.back': 'Zurück',
  'auth.enrol.setUpSigned': 'Du bist eingerichtet und angemeldet',
  'auth.enrol.keepTheseRecoveryCodes': 'Bewahre diese Wiederherstellungscodes sicher auf, getrennt von deinem Handy. Jeder funktioniert einmal, falls du deine Authenticator-App verlierst. Sie werden nur jetzt angezeigt.',
  'auth.enrol.closePageCarryPanel': 'Du kannst diese Seite schließen oder zum Panel weitergehen.',
  'auth.enrol.recoveryCodes': 'Wiederherstellungscodes',
  'auth.enrol.allRecoveryCodes': 'Alle Wiederherstellungscodes',
  'auth.enrol.iSavedThemCarry': 'Ich habe sie gespeichert, weiter',
} satisfies Record<keyof typeof en, string>

register('de', messages)
