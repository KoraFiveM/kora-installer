import { register } from '../../index'
import type en from '../../en/servers/versionsCard'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.versionsCard.whichVersions': 'Welche Versionen',
  'servers.versionsCard.resourcesHereValueSets': 'Die Resources hier sind {style, select, clone {einfache Kopien der Originale} other {deine eigenen Kopien auf GitHub}}. Das legt fest, welchen Versionen der Server folgt.',
  'servers.versionsCard.whichVersionsServerFollows': 'Welchen Versionen der Server folgt',
  'servers.versionsCard.releasedVersionsRecommendedSta': 'Veröffentlichte Versionen (empfohlen, stabil)',
  'servers.versionsCard.eachResourcePinnedTested': 'Jede Resource ist auf ein getestetes Release festgelegt. Du entscheidest, wann du auf ein neueres wechselst.',
  'servers.versionsCard.latestChangesMainBranch': 'Neueste Änderungen (der Main-Branch)',
  'servers.versionsCard.eachResourceFollowsNewest': 'Jede Resource folgt den neuesten Änderungen, auch solchen, die noch nicht veröffentlicht sind.',
  'servers.versionsCard.save': 'Speichern',
} satisfies Record<keyof typeof en, string>

register('de', messages)
