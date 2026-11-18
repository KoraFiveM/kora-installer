import { useQuery } from '@tanstack/react-query'
import { Link, useSearchParams } from 'react-router-dom'
import { api } from '@/api/client'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/servers/joinHelp'

const DEFAULT_GAME_PORT = 30120

const Code = ({ children }: { children: React.ReactNode }) => <code className="break-all rounded bg-muted px-1 font-mono text-xs text-foreground">{children}</code>

function Step({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <Card>
      <CardHeader><CardTitle>{title}</CardTitle></CardHeader>
      <CardContent className="space-y-2 text-sm">{children}</CardContent>
    </Card>
  )
}

/**
 * "Players cannot join my server": the steps in order (server, this computer, home network, firewall, router,
 * provider, outside test), written for someone who has never opened a router page. A page of its own so it can be
 * linked from the reach card and shared. `?server=<name>` fills in that server's own game port.
 */
export default function JoinHelpPage() {
  const [params] = useSearchParams()
  const name = params.get('server')
  const server = useQuery({ queryKey: ['server', name], queryFn: () => api.servers.get(name ?? ''), enabled: !!name, retry: false })
  const known = server.data?.ports.game
  const port = known ?? DEFAULT_GAME_PORT
  const connectCommand = 'connect 127.0.0.1:' + port
  const firewallCommand = 'sudo ufw allow ' + port // i18n-ignore: a command to type, the same in every language
  const lanAddress = '192.168.1.20:' + port

  return (
    <div className="mx-auto max-w-3xl space-y-4">
      <header>
        <h1 className="font-heading text-2xl font-semibold">{t('servers.joinHelp.title')}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{t('servers.joinHelp.intro')}</p>
        <p className="mt-2 text-sm font-medium">{known ? t('servers.joinHelp.portKnown', { port }) : t('servers.joinHelp.portUnknown')}</p>
      </header>
      <Step title={t('servers.joinHelp.runningTitle')}><p>{t('servers.joinHelp.runningBody')}</p></Step>
      <Step title={t('servers.joinHelp.thisComputerTitle')}><p>{rich('servers.joinHelp.thisComputerBody', { command: <Code>{connectCommand}</Code> })}</p></Step>
      <Step title={t('servers.joinHelp.homeNetworkTitle')}><p>{rich('servers.joinHelp.homeNetworkBody', { address: <Code>{lanAddress}</Code> })}</p></Step>
      <Step title={t('servers.joinHelp.firewallTitle')}><p>{rich('servers.joinHelp.firewallBody', { port, command: <Code>{firewallCommand}</Code> })}</p></Step>
      <Step title={t('servers.joinHelp.routerTitle')}>
        <p>{t('servers.joinHelp.routerIntro', { port })}</p>
        <ol className="list-decimal space-y-1 pl-5">
          <li>{rich('servers.joinHelp.routerStep1', { first: <Code>http://192.168.0.1</Code>, second: <Code>http://192.168.1.1</Code> })}</li>
          <li>{t('servers.joinHelp.routerStep2')}</li>
          <li>{t('servers.joinHelp.routerStep3', { port })}</li>
          <li>{t('servers.joinHelp.routerStep4')}</li>
          <li>{t('servers.joinHelp.routerStep5')}</li>
        </ol>
      </Step>
      <Step title={t('servers.joinHelp.providerTitle')}><p>{t('servers.joinHelp.providerBody')}</p></Step>
      <Step title={t('servers.joinHelp.outsideTitle')}><p>{t('servers.joinHelp.outsideBody', { port })}</p></Step>
      <Step title={t('servers.joinHelp.stuckTitle')}><p>{t('servers.joinHelp.stuckBody')}</p></Step>
      <Button asChild variant="default">
        <Link to={name && server.data ? `/servers/${encodeURIComponent(name)}` : '/'}>{t('servers.joinHelp.back')}</Link>
      </Button>
    </div>
  )
}
