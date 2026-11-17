import type { CatalogResource, Group } from '@/api/types'
import { msg, type Msg } from '@/i18n'
import '@/i18n/en/core/catalog'

export const GROUP_ORDER: Group[] = ['pre', 'kora', 'standalone', 'cfx-default', 'default-maps', 'compat', 'props', 'bridges', 'creators', 'other']

export const GROUP_LABEL: Record<Group, Msg> = {
  pre: msg('core.catalog.requiredFirst'),
  kora: msg('core.catalog.core'),
  standalone: msg('core.catalog.standalone'),
  'cfx-default': msg('core.catalog.cfxReDefaults'),
  'default-maps': msg('core.catalog.defaultMaps'),
  compat: msg('core.catalog.compatibility'),
  props: msg('core.catalog.props'),
  bridges: msg('core.catalog.bridges'),
  creators: msg('core.catalog.creators'),
  other: msg('core.catalog.other'),
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
