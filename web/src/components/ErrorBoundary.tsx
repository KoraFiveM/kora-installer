import { Component, type ReactNode } from 'react'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'

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
