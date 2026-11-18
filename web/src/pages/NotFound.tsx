import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { t } from '@/i18n'
import '@/i18n/en/core/notFound'

export default function NotFound() {
  return (
    <div className="mx-auto max-w-xl space-y-4 py-10">
      <h1 className="font-heading text-2xl font-bold">{t('core.notFound.pageFound')}</h1>
      <p className="text-muted-foreground">{t('core.notFound.thereNothingAddressMay')}</p>
      <Button asChild variant="default">
        <Link to="/">{t('core.notFound.backServers')}</Link>
      </Button>
    </div>
  )
}
