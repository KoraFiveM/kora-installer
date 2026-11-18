# Translations

The panel (`web/`) can be shown in more than one language. English (British spelling) is the source of truth; **German
(beta)** ships as the first additional language. This page says how it works, how to add a language, and what is **not**
translated yet.

German is machine-assisted and has **not been reviewed by a native speaker**: it is marked "beta" in the language picker and
must be reviewed before a release that advertises it. It was chosen because German is the largest FiveM community in
Europe and because its long compound words are a good test for layouts.

## What a person sees

- Settings > **Language**, and a small picker on the sign-in and set-up screens. The choice is kept by the browser
  (`localStorage` key `kora.locale`). Without a choice the browser's language list (`navigator.languages`) decides, falling
  back to English.
- `<html lang>` and `dir` follow the language. No right-to-left language ships, but the layout uses logical properties where
  it was touched, and `dir` is set from the language list (`web/src/i18n/locales.ts`).
- Dates, numbers, sizes ("152 kB"), durations, relative times ("3 days ago") and lists come from the platform's `Intl`
  through `fmt` (`web/src/i18n/index.ts`): there is no hand-made formatter left in the app.
- In development and in the mock build (the browser tests) there is a **pseudo-language** ("Pseudo-language (testing)",
  `?lang=en-XA`): English with accents and more letters. Anything that still reads as plain English in it was never put in
  the catalogue, and anything that breaks was too tight for a long language.

## How it is built (no dependency)

A small typed layer in `web/src/i18n/` (a few hundred lines; nothing was added to `package.json`):

| file | what |
| --- | --- |
| `icu.ts` | the message syntax and how a parsed message becomes text |
| `format.ts` | `Intl.PluralRules`, `NumberFormat`, `DateTimeFormat`, `RelativeTimeFormat`, `ListFormat` per language |
| `index.ts` | the current language, `t()`, `msg()`, `fmt`, `setLocale()`, `tById()` |
| `react.tsx` | `rich()` (messages with links or emphasis) and `useLocale()` |
| `locales.ts` | the list of languages, detection from the browser's list |
| `en/<area>/<file>.ts` | the English catalogue, one file per component |
| `de/<area>/<file>.ts` | the German catalogue, mirroring `en/` |
| `generated/` | types generated from the English catalogue |

### Messages

```ts
t('backups.backupsTab.removedCountOldBackup', { count: 3 })   // "Removed 3 old backups."
```

The text is ICU-style (a subset), written in the catalogue:

| | |
| --- | --- |
| argument | `Hello {name}` |
| plural | `{count, plural, =0 {No files} one {# file} other {# files}}` (the language's own categories, from `Intl.PluralRules`) |
| select | `{kind, select, branch {On a branch} other {Somewhere else}}` |
| formats | `{n, number}` `{when, date}` `{when, time}` `{when, datetime}` `{when, relative}` `{items, list}` `{size, bytes}` `{secs, duration}` |
| a piece of the page | `{icon, node}`: an element the code passes in |
| tags | `Read <link>the guide</link>`: filled with functions by `rich()` |
| escapes | `\{` `\}` `\<` `\#` `\\`; an apostrophe is just an apostrophe |

There is **no HTML** anywhere: a tag is a function the component passes (`rich('x.y', { link: (c) => <a href="...">{c}</a> })`),
text in a parameter is never read as markup, and nothing uses `dangerouslySetInnerHTML`. The translation decides where a tag
goes in the sentence; the component decides what it is.

### Ids and types

An id is `<area>.<file>.<name>`, for example `servers.serversPage.colName`. The **area** is the folder under
`web/src/i18n/en/` (`core`, `errors`, `auth`, `servers`, `resources`, `backups`, `install`, `settings`, `system`), the **file** is
the component the text belongs to, and the **name** says what it is. Rules:

- Write the English text so it reads on its own: a translator sees one message, not the page. Do not build a sentence out of
  pieces (`{count} file{s}`): use a plural or a select, so every language can phrase it its own way.
- A new text goes in the English file of the component that uses it. The component imports that file
  (`import '@/i18n/en/servers/serversPage'`), which registers the strings. **That is what keeps the first load small:** the texts
  travel in the same chunk as the page that uses them. `namespaces.test.ts` checks every file imports exactly what it uses.
- Types come from the English catalogue. `generated/params.ts` (the `{arguments}` of every message) and `generated/messages.ts` (the
  list of catalogue files) are regenerated with `KORA_UPDATE_I18N=1 npx vitest run src/i18n/catalogue`; a test fails when they
  are stale. After that a misspelt id, a missing argument or an argument of the wrong kind **fails `tsc`**.
- `t()` takes ids of messages without tags; `rich()` takes the others.
- Tables of words that are module-level constants must not call `t()` while the module loads (the language is not chosen yet):
  keep ids in the table (`msg('core.jobHistory.running')`) and draw them with `t(row.label)`. A `Msg` is not a string, so
  forgetting the `t()` fails `tsc`, and pasting one into a template literal throws in development and tests.

### Keeping it extracted

`src/i18n/hardcoded.test.ts` reads the code with the TypeScript compiler API and **fails when user-facing text is written into the
source**: text between JSX tags, attributes such as `aria-label`/`title`/`placeholder`, strings in expressions that end up on
screen, sentences and lowercase fragments in plain code, labels in tables. A deliberate exception takes a comment on the line
or the line before: `// i18n-ignore: why` (a command shown verbatim, a developer error, text of the server matched by its
English wording).

The same test runs on its own examples, so the rules themselves are tested.

## Adding a language

1. Add it to `LOCALES` in `web/src/i18n/locales.ts` (`code`, its own name, `dir`, `beta: true` until a native speaker reviewed it).
2. Add a loader for it in `index.ts` (`loaders`), and a folder `web/src/i18n/<code>/` with an `index.ts` like `de/index.ts` and one
   file per file in `en/`:
   ```ts
   import { register } from '../../index'
   import type en from '../../en/servers/serversPage'
   const messages = { 'servers.serversPage.colName': '...' } satisfies Record<keyof typeof en, string>
   register('<code>', messages)
   ```
   `satisfies` makes a missing or extra id a compile error.
3. `npx vitest run src/i18n/completeness` checks every message of English exists, with the same arguments and tags, valid syntax,
   no English left over (apart from names such as Kora, txAdmin, FiveM, GitHub, Cfx.re), and that anything in `<code>` or backticks
   (commands, file names) is kept as written. Add the language to `web/e2e/mock/i18n.spec.ts` for the main pages and the
   accessibility scan.
4. Keep the language **beta** until a native speaker has reviewed it.

## Text that comes from the server

Some text is produced in English in Rust and sent as a finished sentence. Translating by matching the English would break the
first time anyone changed a word, so it is moved to **ids**, in steps:

1. **Everything the panel draws itself** (buttons, headings, labels, the names of action buttons, the headings of the system
   check) is translated from kinds and ids.
2. **Problems** (`kora_core::problem::Problem`, the guided errors): next to `message` the server may send an optional,
   backwards-compatible **`i18n`**: `{ "id": "errors.problems.dbStopped", "params": { "name": "MariaDB" } }`
   (`Problem::with_message_id`, `MessageRef`). The panel looks the id up in its catalogue (only ids under `errors.problems.`,
   only messages without tags; the parameters are always text, read as numbers where the message says so) and shows it in the
   person's language. With no id, an unknown id (an older panel, a newer server) or an English panel, it shows the server's
   `message`, as before. So nothing breaks when only one side is updated.
   The English of such a message in the catalogue is exactly what the server writes in `message`, and `problems.test.ts`
   checks every id the Rust code sends exists in the catalogue, in every language, with the parameters the server sends.
   Done so far: `problem::known` (GitHub CLI or Git missing, database stopped, not signed in, missing permissions) and the
   install steps' guided errors (licence key missing, database stopped / needs administrator rights, Git needs administrator
   rights, the GitHub sign-in did not start / complete / finish).
3. **Debug mode:** in a language other than English, a problem **without** an id is shown in the server's English. To find
   those while translating, set `localStorage['kora.i18n.debug'] = '1'`: such text then carries a small amber **EN** mark.
4. **The checklists** (System check, reachability): the lines are typed (`CheckItem.id`, `Layer.id`, `state`), so the headings
   and badges are translated from the ids and states. Their one-sentence summaries are still the server's English.

### How the rest should move to ids

For each kind of server text, the same recipe, one at a time (each step is backwards compatible):

1. Give the sentence an id and name its variable parts. In Rust, `with_message_id("errors.problems.<name>", &[("name", value)])`
   next to the English message (or, for lists of lines such as progress, a `{ "id", "params" }` object next to `message`).
2. Add the English text to `web/src/i18n/en/...` with the same wording, and the translations.
3. Add the id to `problems.test.ts`-style checks so Rust and the catalogue cannot drift.

Not done yet, in the order that would help people most:

- **Problems with no id yet** (most of `Problem::new(...)` in `kora-core`: store, keystore, backups, tools, git and so on). Raise
  them with `.with_message_id(...)` as they are touched.
- **Install progress lines and step titles** (`ProgressEvent.title/detail`), **job status lines** (`ToolInstallEvent.message`,
  backup/update events): send `{ id, params }` next to the text.
- **System check summaries and the reachability layers' `title`, `detail`, `steps`** (they depend on the state and on facts such as
  ports): an id per (layer, state) with parameters.
- **Audit rows** are translated from the operation name already (`settings.store.*`); rows with free text are not.
- **Resource descriptions** from the catalogue, **update/resource states** such as "updated" or "rolling back" and **notes** from
  update reports are the server's English.
- Names given by people (server names, node names, resource titles) are data and are never translated.

## Not translated

- The **command line** (`kora ...` help, output and errors), **logs**, **the support bundle** and **the audit log files**.
- **Server progress lines and notes**, listed above.
- **The desktop app** (`web/src/lib/shell.ts`, `crates/kora-shell`, `crates/kora-desktop`): the tray menu, its labels, notifications
  and the native window title are built from Rust-side English (`crates/kora-shell`'s menu model and notification texts). To
  translate them the shell would take the panel's language (from the browser storage or a setting), and `kora-shell` would
  return message ids that `kora-desktop` renders with its own small catalogue. Not started.
- **Installers** (MSI and `.deb` texts), `README.md` and the docs.
- The **Styleguide** page (a developer page of the mock build).

## Tests

- `icu.test.ts`, `index.test.ts`, `react.test.tsx`: the syntax (plurals in several languages, select, nesting, offset, escapes,
  tags, errors), formatting per language, fallbacks, the pseudo-language, the browser-language choice, `rich()`.
- `catalogue.test.ts`: every English message parses; ids belong to their file; the generated types are current.
- `completeness.test.ts`: each language against English, as above.
- `hardcoded.test.ts`, `namespaces.test.ts`, `problems.test.ts`: nothing hard-coded, imports match uses, server ids exist.
- `web/e2e/mock/i18n.spec.ts`: the picker (Settings and sign-in), German on the main pages with the accessibility scan, a browser
  that asks for German, and the pseudo-language over 14 pages (no plain English left, no sideways overflow, accessibility scan)
  plus two short journeys.
