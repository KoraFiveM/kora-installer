import { register } from '../../index'
import type en from '../../en/servers/joinHelp'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.joinHelp.title': 'Spieler können meinem Server nicht beitreten',
  'servers.joinHelp.intro': 'Geh die Schritte der Reihe nach durch und höre auf, sobald es klappt. Jeder Schritt behebt eine Sache, die Spieler aussperrt. Die Karte „Können Spieler diesen Server erreichen?“ auf der Serverseite prüft das meiste davon für dich.',
  'servers.joinHelp.back': 'Zurück zu den Servern',
  'servers.joinHelp.portKnown': 'Dein Spiel-Port ist {port}.',
  'servers.joinHelp.portUnknown': 'Der Spiel-Port ist 30120, sofern du keinen anderen gewählt hast. Öffne diese Seite von einem Server aus, um seinen Port zu sehen.',
  'servers.joinHelp.runningTitle': '1. Läuft der Server und hört er zu?',
  'servers.joinHelp.runningBody': 'Der Server muss gestartet sein und auf seinem Spiel-Port (TCP und UDP) zuhören. Steht sein Status nicht auf „Läuft“, starte ihn auf der Serverseite. Nutzt schon ein anderes Programm den Port, ändere den Port in den Servereinstellungen und prüfe erneut.',
  'servers.joinHelp.thisComputerTitle': '2. Probiere es zuerst auf diesem Computer',
  'servers.joinHelp.thisComputerBody': 'Drücke in FiveM F8 und gib {command, node} ein. Klappt das nicht, liegt es am Server selbst und nicht an deinem Netzwerk: Lies die Serverkonsole nach Fehlern.',
  'servers.joinHelp.homeNetworkTitle': '3. Probiere es von einem anderen Computer im Heimnetz',
  'servers.joinHelp.homeNetworkBody': 'Verbinde dich mit der Adresse dieses Computers in deinem eigenen Netzwerk, die die Serverseite zeigt (sie sieht aus wie {address, node}). Klappt das, aber Freunde aus dem Internet kommen nicht rein, liegt es an der Firewall oder am Router: Mach weiter.',
  'servers.joinHelp.firewallTitle': '4. Lass den Spiel-Port durch die Firewall',
  'servers.joinHelp.firewallBody': 'Erlaube eingehend TCP und UDP auf Port {port}. Kora macht das bei der Installation eines Servers, oder drücke auf der Karte „Port in der Firewall öffnen“. Windows hat evtl. beim ersten Start des Servers gefragt: Wurde das Fenster mit „Abbrechen“ geschlossen, fehlt die Regel. Unter Linux mit ufw: {command, node}.',
  'servers.joinHelp.routerTitle': '5. Leite den Port im Router weiter',
  'servers.joinHelp.routerIntro': 'Dein Router ist die Haustür deines Heimnetzes. Spieler aus dem Internet kommen nur herein, wenn er Port {port} an diesen Computer weitergibt.',
  'servers.joinHelp.routerStep1': 'Öffne die Einstellungsseite deines Routers (oft {first, node} oder {second, node}; Adresse und Passwort stehen meist auf einem Aufkleber am Router).',
  'servers.joinHelp.routerStep2': 'Suche „Portweiterleitung“ (manchmal „Virtueller Server“, „NAT“ oder „Anwendungen“).',
  'servers.joinHelp.routerStep3': 'Lege eine Regel an: Protokoll TCP und UDP (beide; gibt es nur eine Auswahl, lege zwei Regeln an), äußerer und innerer Port {port}, Ziel ist die Adresse dieses Computers in deinem Netzwerk.',
  'servers.joinHelp.routerStep4': 'Speichern und danach auf der Serverseite „Erneut prüfen“ drücken.',
  'servers.joinHelp.routerStep5': 'Tipp: Gib diesem Computer im Router eine feste Adresse („DHCP-Reservierung“), sonst funktioniert die Regel nicht mehr, wenn sich seine Adresse ändert.',
  'servers.joinHelp.providerTitle': '6. Dein Internetanbieter erlaubt es vielleicht nicht',
  'servers.joinHelp.providerBody': 'Manche Anbieter (oft Mobilfunk, Satellit und einige Glasfaseranschlüsse) teilen eine öffentliche Adresse auf viele Kunden auf (CGNAT), dann kann keine Portweiterleitung funktionieren. Ein Anzeichen: Die Internetadresse, die dein Router zeigt, unterscheidet sich von der, die eine „Wie lautet meine IP“-Webseite zeigt, oder sie beginnt mit 100.64 bis 100.127. Frage den Anbieter nach einer öffentlichen Adresse, betreibe den Server dort, wo es eine gibt, oder nutze Tailscale für eine private Freundesgruppe.',
  'servers.joinHelp.outsideTitle': '7. Teste von außerhalb deines Zuhauses',
  'servers.joinHelp.outsideBody': 'Bitte einen Freund, der nicht in deinem Netzwerk ist, sich mit deiner öffentlichen Adresse und Port {port} zu verbinden. Teste nicht mit deiner eigenen öffentlichen Adresse von zu Hause aus: Viele Router können nicht auf sich selbst zurückleiten, es schlägt also fehl, auch wenn alles stimmt.',
  'servers.joinHelp.stuckTitle': 'Immer noch nichts?',
  'servers.joinHelp.stuckBody': 'Erstelle auf der Serverseite ein Support-Paket (ohne Passwörter und Schlüssel) und hänge es an, wenn du um Hilfe bittest.',
  'servers.joinHelp.link': 'Spieler können immer noch nicht beitreten? Lies die Anleitung zur Fehlersuche',
} satisfies Record<keyof typeof en, string>

register('de', messages)
