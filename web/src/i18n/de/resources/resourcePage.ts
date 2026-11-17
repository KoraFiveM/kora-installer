import { register } from '../../index'
import type en from '../../en/resources/resourcePage'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'resources.resourcePage.couldResource': '{resource} konnte nicht: {a}.',
  'resources.resourcePage.arrowleftNodeResourcesName': '{arrowLeft, node} Resources von {name}',
  'resources.resourcePage.pinnedReleaseSlice': ' · auf Release {slice} festgelegt',
  'resources.resourcePage.nodeNodeResourcesFolder': '{node, node} · resources/{folder}{value}',
  'resources.resourcePage.actions': 'Aktionen',
  'resources.resourcePage.whichVersionCode': 'Welche Version des Codes',
  'resources.resourcePage.stopId': '{id} stoppen?',
  'resources.resourcePage.restartId': '{id} neu starten?',
  'resources.resourcePage.resourceStopsEveryoneServer': 'Die Resource wird für alle auf dem Server gestoppt, bis sie wieder gestartet wird. Andere Resources, die sie brauchen, funktionieren vielleicht nicht mehr. Der Server selbst läuft weiter.',
  'resources.resourcePage.resourceStopsStartsAgain': 'Die Resource wird gestoppt und startet wieder. Spieler bemerken vielleicht eine kurze Unterbrechung bei dem, was sie tut. Der Server selbst läuft weiter.',
  'resources.resourcePage.stopResource': 'Resource stoppen',
  'resources.resourcePage.restartResource': 'Resource neu starten',
} satisfies Record<keyof typeof en, string>

register('de', messages)
