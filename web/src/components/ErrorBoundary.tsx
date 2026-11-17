import { Component, type ReactNode } from 'react'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { isChunkLoadError } from '@/lib/chunkError'

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
          <h1 className="font-heading text-2xl font-bold">This page needs to be reloaded</h1>
          <Alert tone="warning">Kora was probably updated while this page was open, so the part of the page you asked for is not there any more. Your servers are not affected. Reload the page to get the new version.</Alert>
          <div className="flex gap-2">
            <Button variant="default" onClick={() => window.location.reload()}>Reload the page</Button>
          </div>
        </div>
      )
    }
    return (
      <div className="mx-auto max-w-xl space-y-4 py-10">
        <h1 className="font-heading text-2xl font-bold">Something went wrong</h1>
        <Alert tone="danger">This screen hit a problem and could not be shown. Your servers are not affected.</Alert>
        <p className="font-mono text-xs text-muted-foreground">{this.state.error.message}</p>
        <div className="flex gap-2">
          <Button onClick={() => this.setState({ error: null })}>Try again</Button>
          <Button variant="default" onClick={() => window.location.assign('/')}>Back to your servers</Button>
        </div>
      </div>
    )
  }
}
