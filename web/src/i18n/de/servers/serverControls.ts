import { register } from '../../index'
import type en from '../../en/servers/serverControls'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.serverControls.folderMissing': 'Ordner fehlt',
  'servers.serverControls.startName': '{name} starten',
  'servers.serverControls.playNodeStart': '{play, node} Starten ',
  'servers.serverControls.stopName': '{name} stoppen',
  'servers.serverControls.squareNodeStop': '{square, node} Stoppen ',
  'servers.serverControls.restartName': '{name} neu starten',
  'servers.serverControls.rotatecwNodeRestart': '{rotateCw, node} Neu starten ',
  'servers.serverControls.restartName2': '{name} neu starten?',
  'servers.serverControls.everyonePlayingServerDisconnec': 'Alle, die auf diesem Server spielen, werden für etwa eine Minute getrennt, während er neu startet.',
  'servers.serverControls.restartServer': 'Server neu starten',
} satisfies Record<keyof typeof en, string>

register('de', messages)
