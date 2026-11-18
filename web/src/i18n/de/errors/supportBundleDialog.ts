import { register } from '../../index'
import type en from '../../en/errors/supportBundleDialog'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'errors.supportBundleDialog.createSupportBundle': 'Support-Paket erstellen',
  'errors.supportBundleDialog.oneFileHandWhoever': 'Eine Datei, die du der Person geben kannst, die dir hilft, ohne deine Geheimnisse.',
  'errors.supportBundleDialog.accountRoleCannotMake': 'Dein Konto ({role}) kann kein Support-Paket erstellen. Frag einen Administrator.',
  'errors.supportBundleDialog.koraNeverUploadsFile': 'Kora lädt diese Datei nirgendwohin hoch. Sie wird auf diesem Computer gespeichert und verlässt ihn nur, wenn du sie selbst verschickst. Lizenzschlüssel, Datenbank-Anmeldungen, txAdmin-Konten und -Tokens, Webhook-Adressen, Git-Tokens und Passwörter werden vorher entfernt.',
  'errors.supportBundleDialog.koraCollectItsVersion': 'Kora sammelt seine Version, dein System, die Systemprüfung, Werkzeugversionen, die Einstellungen deiner Server (ohne Geheimnisse), letzte Jobs und Probleme, das Aktivitätsprotokoll, Installations- und Update-Logs, den Zustand von Dienst und Firewall sowie den Speicherplatz. Du siehst alles, bevor es gespeichert wird.',
  'errors.supportBundleDialog.prepareShowMeWhat': 'Vorbereiten und zeigen, was darin ist',
  'errors.supportBundleDialog.preparing': 'Wird vorbereitet…',
  'errors.supportBundleDialog.whatFileHoldSize': 'Das enthält die Datei ({size}). Klicke auf eine Datei, um sie genau so zu lesen, wie sie gespeichert wird.',
  'errors.supportBundleDialog.filesSupportBundle': 'Dateien im Support-Paket',
  'errors.supportBundleDialog.redactionsHiddenJoin': '{redactions} verborgen: {join}',
  'errors.supportBundleDialog.leftOutAltogetherBecause': 'Ganz weggelassen, weil sie Geheimnisse enthalten',
  'errors.supportBundleDialog.spanNodeWhy': '{span, node}: sie {why}',
  'errors.supportBundleDialog.textOpen': 'Text von {open}',
  'errors.supportBundleDialog.empty': '(leer)',
  'errors.supportBundleDialog.neverUploadedSavedSpan': '{never_uploaded} Es wird in {span, node} gespeichert.',
  'errors.supportBundleDialog.saveBundle': 'Dieses Paket speichern',
  'errors.supportBundleDialog.cancelSaveNothing': 'Abbrechen, nichts speichern',
  'errors.supportBundleDialog.saved': 'Gespeichert',
  'errors.supportBundleDialog.sendOneFileWhoever': 'Schicke diese eine Datei an die Person, die dir hilft.',
  'errors.supportBundleDialog.whereSaved': 'Wo es gespeichert wurde',
  'errors.supportBundleDialog.downloadNodeDownload': '{download, node} Herunterladen',
  'errors.supportBundleDialog.close': 'Schließen',
} satisfies Record<keyof typeof en, string>

register('de', messages)
