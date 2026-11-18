import { Component, type ReactNode } from 'react'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { isChunkLoadError } from '@/lib/chunkError'
import { t } from '@/i18n'
import '@/i18n/en/core/errorBoundary'

export class ErrorBoundary extends Component<{ children: ReactNode; resetKey?: string }, { error: Error | null }> {
  state = { error: null as Error | null }
  static getDerivedStateFromError(error: Error) {
    return { error }
  }
  componentDidUpdate(prev: { resetKey?: string }) {
    if (this.state.error && prev.resetKey !== this.props.resetKey) this.setState({ error: null })
  }
  render() {
    if (!this.state.error) return this.props.children
    // A page file that cannot be loaded (Kora was updated while this tab was open, or the connection dropped): only a
    // reload helps, because React keeps the failed import. Say that, instead of a blank page or a useless "Try again".
    if (isChunkLoadError(this.state.error)) {
      return (
        <div className="mx-auto max-w-xl space-y-4 py-10" data-testid="chunk-error">
          <h1 className="font-heading text-2xl font-bold">{t('core.errorBoundary.pageNeedsReloaded')}</h1>
          <Alert tone="warning">{t('core.errorBoundary.koraProbablyUpdatedWhile')}</Alert>
          <div className="flex gap-2">
            <Button variant="default" onClick={() => window.location.reload()}>{t('core.errorBoundary.reloadPage')}</Button>
          </div>
        </div>
      )
    }
    return (
      <div className="mx-auto max-w-xl space-y-4 py-10">
        <h1 className="font-heading text-2xl font-bold">{t('core.errorBoundary.somethingWentWrong')}</h1>
        <Alert tone="danger">{t('core.errorBoundary.screenHitProblemCould')}</Alert>
        <p className="font-mono text-xs text-muted-foreground">{this.state.error.message}</p>
        <div className="flex gap-2">
          <Button onClick={() => this.setState({ error: null })}>{t('core.errorBoundary.tryAgain')}</Button>
          <Button variant="default" onClick={() => window.location.assign('/')}>{t('core.errorBoundary.backServers')}</Button>
        </div>
      </div>
    )
  }
}
