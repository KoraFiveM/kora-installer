import { ExternalLink } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { t } from '@/i18n'
import '@/i18n/en/settings/contribute'

const REPO = 'https://github.com/KoraFiveM/kora-installer'

/** Settings > Contribute: the three steps to open a pull request on the Kora repository. */
export function ContributeCard() {
  return (
    <Card>
      <CardHeader><CardTitle>{t('settings.contribute.title')}</CardTitle></CardHeader>
      <CardContent className="space-y-3 text-sm">
        <p>{t('settings.contribute.intro')}</p>
        <ol className="list-decimal space-y-1 pl-5">
          <li>{t('settings.contribute.step1')}</li>
          <li>{t('settings.contribute.step2')}</li>
          <li>{t('settings.contribute.step3')}</li>
        </ol>
        <div className="flex flex-wrap gap-2">
          <Button asChild variant="default"><a href={REPO} target="_blank" rel="noopener noreferrer">{t('settings.contribute.openRepo')} <ExternalLink aria-hidden /></a></Button>
          <Button asChild><a href={`${REPO}/blob/main/CONTRIBUTING.md`} target="_blank" rel="noopener noreferrer">{t('settings.contribute.guide')} <ExternalLink aria-hidden /></a></Button>
        </div>
      </CardContent>
    </Card>
  )
}
