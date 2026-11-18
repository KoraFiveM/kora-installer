import type { ServerDetail } from '@/api/types'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { GithubSignIn } from '@/components/GithubSignIn'
import { BranchesPanel } from './branches/BranchesPanel'
import { t } from '@/i18n'
import '@/i18n/en/resources/githubTab'

export function GithubTab({ server: s }: { server: ServerDetail }) {
  return (
    <div className="grid gap-5 lg:grid-cols-2">
      <Card className="lg:col-span-2"><CardHeader><CardTitle>{t('resources.githubTab.githubAccount')}</CardTitle></CardHeader><CardContent><GithubSignIn /></CardContent></Card>
      <div className="lg:col-span-2"><BranchesPanel server={s} /></div>
      <Card className="lg:col-span-2">
        <CardHeader><CardTitle>{t('resources.githubTab.pullRequestsIssuesChanges')}</CardTitle></CardHeader>
        <CardContent className="text-sm text-muted-foreground">{t('resources.githubTab.theseLiveEachResources')}</CardContent>
      </Card>
    </div>
  )
}
