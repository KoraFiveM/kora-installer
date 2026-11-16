import type { CatalogResource, Group } from '@/api/types'

export const GROUP_ORDER: Group[] = ['pre', 'kora', 'standalone', 'cfx-default', 'default-maps', 'compat', 'props', 'bridges', 'creators', 'other']

export const GROUP_LABEL: Record<Group, string> = {
  pre: 'Required first',
  kora: 'Core',
  standalone: 'Standalone',
  'cfx-default': 'Cfx.re defaults',
  'default-maps': 'Default maps',
  compat: 'Compatibility',
  props: 'Props',
  bridges: 'Bridges',
  creators: 'Creators',
  other: 'Other',
}

export function groupResources(list: CatalogResource[], query: string): [Group, CatalogResource[]][] {
  const q = query.trim().toLowerCase()
  const match = (r: CatalogResource) => !q || [r.id, r.title, r.description].some((t) => t?.toLowerCase().includes(q))
  const out: [Group, CatalogResource[]][] = []
  for (const g of GROUP_ORDER) {
    const items = list.filter((r) => r.group === g && r.kind !== 'internal' && match(r))
    if (items.length) out.push([g, items])
  }
  return out
}
