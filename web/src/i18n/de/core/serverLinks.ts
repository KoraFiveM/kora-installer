import { register } from '../../index'
import type en from '../../en/core/serverLinks'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'core.serverLinks.reachedPanelThroughTunnel': 'Du hast dieses Panel über einen Tunnel erreicht. txAdmin öffnet sich über das Panel auf Port {gw}. Füge deshalb -L {gw}:localhost:{gw} zu deinem ssh-Befehl hinzu und drücke dann den Knopf.',
  'core.serverLinks.opensTxadminThroughPanel': 'Öffnet txAdmin über dieses Panel (du bleibst mit deiner Panel-Anmeldung angemeldet), auf Port {gw} von {host}. Wenn die Seite nicht lädt, blockiert eine Firewall diesen Port: Erlaube TCP-Port {gw} für die Geräte, die das Panel nutzen.',
  'core.serverLinks.reachedPanelThroughTunnelOwn': 'Du hast dieses Panel über einen Tunnel erreicht. txAdmin öffnet sich über das Panel auf einem eigenen Port. Füge deshalb eine -L-Option für diesen Port zu deinem ssh-Befehl hinzu und drücke dann den Knopf.',
  'core.serverLinks.opensTxadminThroughPanelOwn': 'Öffnet txAdmin über dieses Panel (du bleibst mit deiner Panel-Anmeldung angemeldet), auf einem eigenen Port von {host}. Wenn die Seite nicht lädt, blockiert eine Firewall diesen Port: Erlaube ihn für die Geräte, die das Panel nutzen.',
  'core.serverLinks.addressOwnNetworkPlayers': 'Das ist die Adresse in deinem eigenen Netzwerk: Spieler im selben Netzwerk (oder WLAN) können sie benutzen. Spieler im Internet brauchen deine öffentliche Adresse mit diesem Port, und der Port muss in deiner Firewall offen und an deinem Router weitergeleitet sein (oder bei deinem Hosting-Anbieter geöffnet werden). Die Prüfung unten („Können Spieler diesen Server erreichen?“) findet deine öffentliche Adresse, sieht sich jeden dieser Schritte an und kann die Portweiterleitung für dich einrichten.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
