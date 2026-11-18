import { register } from '../../index'
import type en from '../../en/servers/reachCard'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.reachCard.fromInternetNeedsRouter': 'Aus dem Internet (setzt voraus, dass die Router- und Firewall-Schritte oben stimmen)',
  'servers.reachCard.internetAddress': 'Internetadresse',
  'servers.reachCard.cfxReJoinLink': 'Cfx.re-Beitrittslink',
  'servers.reachCard.contactWhat': 'Dabei wird {what} kontaktiert',
  'servers.reachCard.choiceRememberedTurnOff': 'Deine Wahl wird gemerkt; du kannst sie unten auf dieser Karte wieder ausschalten.',
  'servers.reachCard.notNow': 'Jetzt nicht',
  'servers.reachCard.justOnce': 'Nur dieses Mal',
  'servers.reachCard.letKoraChangeRouter': 'Kora deinen Router ändern lassen?',
  'servers.reachCard.turnPortForwarding': 'Portweiterleitung einschalten',
  'servers.reachCard.playersReachServer': 'Können Spieler diesen Server erreichen?',
  'servers.reachCard.koraChecksFourThings': 'Kora prüft nacheinander vier Dinge: Hört der Server zu, lässt die Firewall dieses Computers Spieler herein, leitet dein Router sie hierher, und kann das Internet ihn erreichen. Es sieht sich nur diesen Computer und deinen Router an. Außerhalb wird nichts kontaktiert, solange du es nicht erlaubst. ',
  'servers.reachCard.checkAgain': 'Erneut prüfen',
  'servers.reachCard.checkNow': 'Jetzt prüfen',
  'servers.reachCard.lookingServerFirewallRouter': 'Server, Firewall und Router werden angesehen (ein paar Sekunden)…',
  'servers.reachCard.automaticPortForwarding': 'Automatische Portweiterleitung ist an.',
  'servers.reachCard.turnOff': 'Ausschalten',
  'servers.reachCard.addressesGivePeople': 'Adressen, die du weitergeben kannst',
  'servers.reachCard.addressOwnNetwork': 'Adresse im eigenen Netzwerk',
  'servers.reachCard.addressInternet': 'Adresse im Internet',
  'servers.reachCard.outsideLookupsKoraMay': 'Abfragen nach außen, die Kora für diese Prüfung machen darf (aus, solange du sie nicht erlaubst)',
  'servers.reachCard.cfxResPublicServer': 'Öffentliche Serverliste von Cfx.re',
  'servers.reachCard.asksFrontendCfxservicesNet': 'Fragt frontend.cfx-services.net nach dem eigenen Beitrittscode dieses Servers.',
  'servers.reachCard.publicaddressWebsiteApiIpify': 'Eine Webseite für öffentliche Adressen (api.ipify.org)',
  'servers.reachCard.onlyUsedWhenRouter': 'Wird nur benutzt, wenn der Router Kora die öffentliche Adresse nicht nennen kann. Sie sieht die öffentliche Adresse dieses Computers.',
  'servers.reachCard.checkAskedWhatLearned': 'Diese Prüfung hat {what} gefragt; sie hat {learned} erfahren.',
  'servers.reachCard.probablyOk': 'Wahrscheinlich in Ordnung',
  'servers.reachCard.problem': 'Problem',
  'servers.reachCard.notKnown': 'Unbekannt',
  'servers.reachCard.routerProgramsReconfigureSmall': 'Ein Router, den Programme umkonfigurieren können, ist ein kleines Risiko: Jedes Programm in deinem Netzwerk kann dasselbe tun. Wenn du das einschaltest, darf Kora es nur für den Spiel-Port dieses Servers tun (TCP und UDP): Es behält die Regel, solange der Server läuft, und entfernt sie, wenn der Server anhält.',
  'servers.reachCard.cfxResPublicServer2': 'Öffentliche Serverliste von Cfx.re (frontend.cfx-services.net)',
  'servers.reachCard.koraSendsServersJoin': 'Kora sendet den Beitrittscode deines Servers (den aus seinem eigenen Log). Cfx.re kennt ihn schon; Kora erfährt, ob du gelistet bist und welche Adresse Cfx.re hat.',
  'servers.reachCard.allowCheckAgain': 'Erlauben und erneut prüfen',
  'servers.reachCard.websiteSeesComputersPublic': 'Diese Webseite sieht die öffentliche Adresse dieses Computers und dass eine Anfrage gestellt wurde. Kora erfährt deine öffentliche Adresse.',
  'servers.reachCard.ok': 'OK',
} satisfies Record<keyof typeof en, string>

register('de', messages)
