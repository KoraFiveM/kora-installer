import { register } from '../../index'

const messages = {
  'servers.notificationsTab.discordNotifications': 'Discord notifications',
  'servers.notificationsTab.koraPostDiscordChannel': 'Kora can post to a Discord channel when updates finish or something needs your attention. In Discord, open the channel settings, choose Integrations, then Webhooks, and copy the webhook address.',
  'servers.notificationsTab.currentWebhookCodeNode': 'Current webhook: {code, node}',
  'servers.notificationsTab.sendTestMessage': 'Send a test message',
  'servers.notificationsTab.removeWebhook': 'Remove webhook',
  'servers.notificationsTab.noWebhookSet': 'No webhook is set.',
  'servers.notificationsTab.replaceWebhookAddress': 'Replace the webhook address',
  'servers.notificationsTab.webhookAddress': 'Webhook address',
  'servers.notificationsTab.addressKeptSecretNever': 'The address is kept secret: it is never shown in full again.',
  'servers.notificationsTab.httpsDiscordComApi': 'https://discord.com/api/webhooks/...',
  'servers.notificationsTab.replaceWebhook': 'Replace webhook',
  'servers.notificationsTab.saveWebhook': 'Save webhook',
  'servers.notificationsTab.testMessageSentCheck': 'Test message sent. Check your Discord channel.',
  'servers.notificationsTab.removeWebhook2': 'Remove the webhook?',
  'servers.notificationsTab.alertsStopBeingSent': 'Alerts stop being sent. You will need the address again to turn them back on.',
} as const

register('en', messages)
export default messages
