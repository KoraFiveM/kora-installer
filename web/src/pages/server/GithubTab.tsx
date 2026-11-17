import type { ServerDetail } from '@/api/types'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { GithubSignIn } from '@/components/GithubSignIn'
import { BranchesPanel } from './branches/BranchesPanel'

export function GithubTab({ server: s }: { server: ServerDetail }) {
  return (
    <div className="grid gap-5 lg:grid-cols-2">
      <Card className="lg:col-span-2"><CardHeader><CardTitle>GitHub account</CardTitle></CardHeader><CardContent><GithubSignIn /></CardContent></Card>
      <div className="lg:col-span-2"><BranchesPanel server={s} /></div>
      <Card className="lg:col-span-2">
        <CardHeader><CardTitle>Pull requests, issues, changes and releases</CardTitle></CardHeader>
        <CardContent className="text-sm text-muted-foreground">
          These live on each resource's own page: open one from the list above, or from the Resources tab. There you can see what changed, save and push it, open a pull request, try or review other people's, report and comment on issues, and see releases.
        </CardContent>
      </Card>
    </div>
  )
}
