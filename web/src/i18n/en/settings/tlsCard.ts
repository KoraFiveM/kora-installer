import { register } from '../../index'

const messages = {
  'settings.tlsCard.daysLeftDaysLeft': '{days, plural, one {# day left} other {# days left}}',
  'settings.tlsCard.expired': 'expired',
  'settings.tlsCard.panelUsesValueBadge': 'The panel uses {source}. {badge, node}',
  'settings.tlsCard.validUntil': 'Valid until',
  'settings.tlsCard.names': 'Names',
  'settings.tlsCard.fingerprintSha': 'Fingerprint (SHA-256)',
  'settings.tlsCard.localAuthority': 'Local authority',
  'settings.tlsCard.compareFingerprintWhatBrowser': "Compare the fingerprint with what your browser shows for the certificate (click the padlock, then the certificate). Plain HTTP on this computer's own address stays as it is: no warnings there.",
  'settings.tlsCard.noHttpsCertificate': 'No HTTPS certificate',
  'settings.tlsCard.panelServesPlainHttp': 'The panel serves plain HTTP. That is fine on this computer and inside Tailscale. On a home network or the internet, set up HTTPS: run <code>kora panel tls self-signed</code> for a certificate made on this computer, or <code2>kora panel tls set --cert \\<file> --key \\<file></code2> for your own. ',
  'settings.tlsCard.httpsProblem': 'HTTPS problem',
  'settings.tlsCard.replaceFilesPanelReads': 'Replace the files; the panel reads them again by itself.',
  'settings.tlsCard.koraRenewsItselfUse': 'Kora renews it by itself; use Renew now if it does not.',
  'settings.tlsCard.certificateEndsSoonValue': 'The certificate ends soon. {value}',
  'settings.tlsCard.reverseProxyTrustedCode': 'A reverse proxy is trusted (<code>kora panel proxy status</code>).',
  'settings.tlsCard.renewNow': 'Renew now',
  'settings.tlsCard.showAuthoritysCertificate': "Show the authority's certificate",
  'settings.tlsCard.installCertificateAuthorityOnc': 'Install this certificate authority once, so browsers trust the panel. Compare its fingerprint with the one above first. Or use <code>kora panel tls export-ca</code> to save it as a file.',
  'settings.tlsCard.certificateAuthorityPem': 'Certificate authority (PEM)',
  'settings.tlsCard.windowsEdgeChrome': 'Windows (Edge, Chrome)',
  'settings.tlsCard.doubleclickSavedFileChoose': 'Double-click the saved file, choose Install Certificate, Current User, then place it in Trusted Root Certification Authorities. Restart the browser.',
  'settings.tlsCard.doubleclickFileAddKeychain': 'Double-click the file to add it to Keychain Access, open it there and set "When using this certificate" to Always Trust.',
  'settings.tlsCard.linuxChromeChromiumFirefox': 'Linux (Chrome, Chromium) and Firefox',
  'settings.tlsCard.browserSettingsCertificatesAut': 'In the browser settings, Certificates, Authorities: import the file and tick "Trust this CA to identify websites".',
  'settings.tlsCard.sendFileDeviceOpen': 'Send the file to the device and open it, then switch it on under Certificate Trust Settings (iPhone) or install it as a CA certificate (Android).',
  'settings.tlsCard.sourceProvided': 'a certificate you provided',
  'settings.tlsCard.sourceTailscale': 'a Tailscale certificate',
  'settings.tlsCard.sourceSelfSigned': 'a self-signed certificate made by Kora',
  'settings.tlsCard.sourceOther': 'a certificate',
  'settings.tlsCard.platformMac': 'macOS (Safari, Chrome)',
  'settings.tlsCard.platformMobile': 'iPhone, iPad, Android',
} as const

register('en', messages)
export default messages
