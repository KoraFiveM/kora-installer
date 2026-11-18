// German (beta). One lazily loaded chunk for the whole language: every file in this folder registers its messages.
// Machine-assisted and not yet reviewed by a native speaker (docs/translations.md). The files mirror ../en/.
import.meta.glob('./*/*.ts', { eager: true })
