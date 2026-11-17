import { register } from '../../index'
import type en from '../../en/resources/changesDialog'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.changesDialog.throwChangesAway': 'Deine Änderungen wegwerfen?',
  'resources.changesDialog.changesSaved': 'Du hast Änderungen, die nicht gespeichert sind',
  'resources.changesDialog.thisResource': 'Diese Resource',
  'resources.changesDialog.countResourcesChangesOwn': '{count} Resources haben eigene Änderungen von dir: {join}.',
  'resources.changesDialog.valueChangesOwnWere': '{resource} hat Änderungen von dir, die nie als Commit gespeichert wurden.',
  'resources.changesDialog.changedFiles': 'Geänderte Dateien',
  'resources.changesDialog.cannotUndone': 'Das lässt sich nicht rückgängig machen',
  'resources.changesDialog.changedFilesGoBack': 'Die geänderten Dateien gehen zurück zu dem, wie sie waren, und neue Dateien, die du in {where} hinzugefügt hast, werden gelöscht. Kora behält keine Kopie.',
  'resources.changesDialog.goBack': 'Zurück',
  'resources.changesDialog.throwMyChangesAway': 'Meine Änderungen wegwerfen',
  'resources.changesDialog.koraSwitchUntilChoose': 'Kora wechselt erst, wenn du dich entscheidest, damit nichts versehentlich verloren geht.',
  'resources.changesDialog.keepMyChangesAside': 'Meine Änderungen beiseitelegen',
  'resources.changesDialog.recommendedTheySetAside': 'Empfohlen. Sie werden sicher beiseitegelegt, und du kannst sie später in diesem Panel zurückholen.',
  'resources.changesDialog.throwMyChangesAway2': 'Meine Änderungen wegwerfen…',
  'resources.changesDialog.askedConfirm': 'Du wirst um eine Bestätigung gebeten.',
  'resources.changesDialog.cancelChangeNothing': 'Abbrechen, nichts ändern',
} satisfies Record<keyof typeof en, string>

register('de', messages)
