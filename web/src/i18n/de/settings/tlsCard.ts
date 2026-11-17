import { register } from '../../index'
import type en from '../../en/settings/tlsCard'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'settings.tlsCard.daysLeftDaysLeft': '{days, plural, one {# Tag übrig} other {# Tage übrig}}',
  'settings.tlsCard.expired': 'abgelaufen',
  'settings.tlsCard.panelUsesValueBadge': 'Das Panel nutzt {source}. {badge, node}',
  'settings.tlsCard.validUntil': 'Gültig bis',
  'settings.tlsCard.names': 'Namen',
  'settings.tlsCard.fingerprintSha': 'Fingerabdruck (SHA-256)',
  'settings.tlsCard.localAuthority': 'Lokale Zertifizierungsstelle',
  'settings.tlsCard.compareFingerprintWhatBrowser': 'Vergleiche den Fingerabdruck mit dem, was dein Browser für das Zertifikat anzeigt (klicke auf das Vorhängeschloss, dann auf das Zertifikat). Normales HTTP auf der eigenen Adresse dieses Computers bleibt, wie es ist: Dort gibt es keine Warnungen.',
  'settings.tlsCard.noHttpsCertificate': 'Kein HTTPS-Zertifikat',
  'settings.tlsCard.panelServesPlainHttp': 'Das Panel liefert normales HTTP. Das ist auf diesem Computer und in Tailscale in Ordnung. Richte in einem Heimnetz oder im Internet HTTPS ein: Führe <code>kora panel tls self-signed</code> für ein auf diesem Computer erstelltes Zertifikat aus oder <code2>kora panel tls set --cert \\<file> --key \\<file></code2> für dein eigenes. ',
  'settings.tlsCard.httpsProblem': 'HTTPS-Problem',
  'settings.tlsCard.replaceFilesPanelReads': 'Ersetze die Dateien; das Panel liest sie von selbst neu ein.',
  'settings.tlsCard.koraRenewsItselfUse': 'Kora erneuert es von selbst; nutze „Jetzt erneuern“, falls nicht.',
  'settings.tlsCard.certificateEndsSoonValue': 'Das Zertifikat läuft bald ab. {value}',
  'settings.tlsCard.reverseProxyTrustedCode': 'Einem Reverse-Proxy wird vertraut (<code>kora panel proxy status</code>).',
  'settings.tlsCard.renewNow': 'Jetzt erneuern',
  'settings.tlsCard.showAuthoritysCertificate': 'Das Zertifikat der Zertifizierungsstelle anzeigen',
  'settings.tlsCard.installCertificateAuthorityOnc': 'Installiere diese Zertifizierungsstelle einmal, damit Browser dem Panel vertrauen. Vergleiche vorher ihren Fingerabdruck mit dem oben. Oder speichere sie mit <code>kora panel tls export-ca</code> als Datei.',
  'settings.tlsCard.certificateAuthorityPem': 'Zertifizierungsstelle (PEM)',
  'settings.tlsCard.windowsEdgeChrome': 'Windows (Edge, Chrome)',
  'settings.tlsCard.doubleclickSavedFileChoose': 'Doppelklicke auf die gespeicherte Datei, wähle „Zertifikat installieren“, „Aktueller Benutzer“ und lege es dann in „Vertrauenswürdige Stammzertifizierungsstellen“ ab. Starte den Browser neu.',
  'settings.tlsCard.doubleclickFileAddKeychain': 'Doppelklicke auf die Datei, um sie zur Schlüsselbundverwaltung hinzuzufügen, öffne sie dort und setze „Bei Verwendung dieses Zertifikats“ auf „Immer vertrauen“.',
  'settings.tlsCard.linuxChromeChromiumFirefox': 'Linux (Chrome, Chromium) und Firefox',
  'settings.tlsCard.browserSettingsCertificatesAut': 'In den Browsereinstellungen unter Zertifikate, Zertifizierungsstellen: Importiere die Datei und hake „Dieser CA vertrauen, um Websites zu identifizieren“ an.',
  'settings.tlsCard.sendFileDeviceOpen': 'Sende die Datei an das Gerät und öffne sie, schalte sie dann unter Zertifikatsvertrauenseinstellungen ein (iPhone) oder installiere sie als CA-Zertifikat (Android).',
  'settings.tlsCard.sourceProvided': 'ein von dir bereitgestelltes Zertifikat',
  'settings.tlsCard.sourceTailscale': 'ein Tailscale-Zertifikat',
  'settings.tlsCard.sourceSelfSigned': 'ein von Kora erstelltes selbstsigniertes Zertifikat',
  'settings.tlsCard.sourceOther': 'ein Zertifikat',
  'settings.tlsCard.platformMac': 'macOS (Safari, Chrome)',
  'settings.tlsCard.platformMobile': 'iPhone, iPad, Android',
} satisfies Record<keyof typeof en, string>

register('de', messages)
