import { register } from '../../index'
import type en from '../../en/servers/notificationsTab'

// German (beta): machine-assisted, to be checked by a native speaker before release.
const messages = {
  'servers.notificationsTab.discordNotifications': 'Discord-Benachrichtigungen',
  'servers.notificationsTab.koraPostDiscordChannel': 'Kora kann in einem Discord-Kanal posten, wenn Updates abgeschlossen sind oder etwas deine Aufmerksamkeit braucht. Öffne in Discord die Kanaleinstellungen, wähle Integrationen, dann Webhooks, und kopiere die Webhook-Adresse.',
  'servers.notificationsTab.currentWebhookCodeNode': 'Aktueller Webhook: {code, node}',
  'servers.notificationsTab.sendTestMessage': 'Testnachricht senden',
  'servers.notificationsTab.removeWebhook': 'Webhook entfernen',
  'servers.notificationsTab.noWebhookSet': 'Es ist kein Webhook gesetzt.',
  'servers.notificationsTab.replaceWebhookAddress': 'Die Webhook-Adresse ersetzen',
  'servers.notificationsTab.webhookAddress': 'Webhook-Adresse',
  'servers.notificationsTab.addressKeptSecretNever': 'Die Adresse wird geheim gehalten: Sie wird nie wieder vollständig angezeigt.',
  'servers.notificationsTab.httpsDiscordComApi': 'https://discord.com/api/webhooks/...',
  'servers.notificationsTab.replaceWebhook': 'Webhook ersetzen',
  'servers.notificationsTab.saveWebhook': 'Webhook speichern',
  'servers.notificationsTab.testMessageSentCheck': 'Testnachricht gesendet. Sieh in deinem Discord-Kanal nach.',
  'servers.notificationsTab.removeWebhook2': 'Den Webhook entfernen?',
  'servers.notificationsTab.alertsStopBeingSent': 'Es werden keine Warnungen mehr gesendet. Du brauchst die Adresse wieder, um sie erneut einzuschalten.',
} satisfies Record<keyof typeof en, string>

register('de', messages)
