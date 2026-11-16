import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'

export default function NotFound() {
  return (
    <div className="mx-auto max-w-xl space-y-4 py-10">
      <h1 className="font-heading text-2xl font-bold">Page not found</h1>
      <p className="text-muted-foreground">There is nothing at this address. It may have been removed or typed wrongly.</p>
      <Button asChild variant="default">
        <Link to="/">Back to your servers</Link>
      </Button>
    </div>
  )
}
