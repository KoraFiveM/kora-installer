import { CheckCircle2, Circle } from 'lucide-react'
import { Link } from 'react-router-dom'
import { Alert } from '@/components/ui/alert'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { LicenceKeysCard } from '@/components/LicenceKeys'
import { GithubSignIn } from '@/components/GithubSignIn'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/install/installWizard'
import '@/i18n/en/core/setupGate'

function Step({ n, done, title, hint }: { n: number; done: boolean | null; title: string; hint: string }) {
  const Icon = done ? CheckCircle2 : Circle
  return (
    <li className="flex gap-3">
      <Icon className={done ? 'mt-0.5 size-5 shrink-0 text-success' : 'mt-0.5 size-5 shrink-0 text-muted-foreground'} aria-hidden />
      <div>
        <div className="font-semibold">{t('core.setupGate.stepTitle', { n, title })} <span className="sr-only">{done ? t('core.setupGate.done') : t('core.setupGate.todo')}</span></div>
        <p className="text-sm text-muted-foreground">{hint}</p>
      </div>
    </li>
  )
}

/** Shown instead of a page until a Cfx.re licence key is saved and GitHub is signed in. Both are done right here. */
export function SetupGate({ cfxReady, githubReady }: { cfxReady: boolean | null; githubReady: boolean | null }) {
  return (
    <div className="mx-auto max-w-3xl space-y-5">
      <h1 className="font-heading text-2xl font-bold">{t('core.setupGate.title')}</h1>
      <Alert tone="info">{t('core.setupGate.intro')}</Alert>
      {/* A way out that needs neither: a server that already exists is added without a licence key or GitHub. */}
      <p className="text-sm text-muted-foreground">{rich('install.installWizard.alreadyServerComputerLink', { link: (c) => <Link to="/?add=1" className="text-info underline underline-offset-4">{c}</Link> })}</p>
      <Card>
        <CardHeader><CardTitle>{t('core.setupGate.checklist')}</CardTitle></CardHeader>
        <CardContent>
          <ol className="space-y-4">
            <Step n={1} done={cfxReady} title={t('core.setupGate.cfxTitle')} hint={t('core.setupGate.cfxHint')} />
            <Step n={2} done={githubReady} title={t('core.setupGate.githubTitle')} hint={t('core.setupGate.githubHint')} />
          </ol>
        </CardContent>
      </Card>
      {cfxReady === false && <LicenceKeysCard />}
      {githubReady === false && <Card><CardHeader><CardTitle>{t('core.setupGate.githubTitle')}</CardTitle></CardHeader><CardContent><GithubSignIn /></CardContent></Card>}
    </div>
  )
}
