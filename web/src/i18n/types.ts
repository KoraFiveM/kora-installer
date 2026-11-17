// The ids and parameters of every message, from the English catalogue (the source of truth).
// Adding an area: import its module here (type only: nothing is bundled) and add it to `Messages`.
import type core from './en/core'
import type { MessageParams as Generated, RichIds } from './generated/params'

export type Messages = typeof core

export type MessageId = keyof Messages & string
export type MessageParams = Generated
export type RichId = RichIds & MessageId
/** Ids whose text has no <tags>: what `t()` accepts. */
export type PlainId = Exclude<MessageId, RichId>
