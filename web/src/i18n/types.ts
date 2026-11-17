// The ids and parameters of every message, from the English catalogue (the source of truth).
// Adding an area: import its module here (type only: nothing is bundled) and add it to `Messages`.
import type core from './en/core'
import type errors from './en/errors'
import type auth from './en/auth'
import type servers from './en/servers'
import type resources from './en/resources'
import type backups from './en/backups'
import type install from './en/install'
import type settings from './en/settings'
import type system from './en/system'
import type { MessageParams as Generated, RichIds } from './generated/params'

export type Messages = typeof core & typeof errors & typeof auth & typeof servers & typeof resources & typeof backups & typeof install & typeof settings & typeof system

export type MessageId = keyof Messages & string
export type MessageParams = Generated
export type RichId = RichIds & MessageId
/** Ids whose text has no <tags>: what `t()` accepts. */
export type PlainId = Exclude<MessageId, RichId>
