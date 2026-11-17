import { Fragment, createElement, useSyncExternalStore, type ReactNode } from 'react'
import { locale, pieces, subscribe, type MessageParams, type RichId } from './index'
import type { Value } from './icu'

/** A tag in a message (`<link>text</link>`) is filled with a function that wraps its text. */
export type Tag = (children: ReactNode) => ReactNode

/** The current language, as a React value: a component that reads it re-renders when the language changes. */
export function useLocale(): string {
  return useSyncExternalStore(subscribe, locale)
}

const wrap = (parts: unknown[]): ReactNode => parts.map((p, i) => (typeof p === 'string' ? p : createElement(Fragment, { key: i }, p as ReactNode)))

/**
 * A message with <tags> and arguments: `rich('x.help', { link: (c) => <a href="...">{c}</a>, name })`.
 * The translation places the tags; the component decides what they are. No HTML is parsed anywhere.
 */
export function rich<K extends RichId>(id: K, params: MessageParams[K]): ReactNode {
  const p: Record<string, Value> = {}
  for (const [k, v] of Object.entries(params as Record<string, unknown>)) {
    p[k] = typeof v === 'function' ? ((children: unknown[]) => (v as Tag)(wrap(children))) : (v as Value)
  }
  return wrap(pieces(id, p))
}
