# Kora's data file (the store)

From version 0.8 Kora keeps everything it remembers about itself in **one database file** in the Kora folder:

| | |
|---|---|
| Linux | `~/.kora/store/kora.db` (or `$KORA_HOME/store/kora.db`) |
| Windows | `%LOCALAPPDATA%\Kora\store\kora.db` |

It is a SQLite database (bundled with Kora: nothing to install). It replaces the small JSON files that were each rewritten whole, so the
panel and a `kora` command running at the same moment can no longer overwrite each other's change.

## What is in it, and what stays a file

**In the data file:** the server list (was `servers.json`), the saved licence keys (`secrets.json`), what Kora remembers about you
(`profile.json`), the answers of an install that stopped (`last-install.json`, `paused-installs/`), remote-access settings and users
(`panel.json`), the nodes (`hub.json`), the **job history** (new: it survives a restart) and the **audit log** (new).

**Still files**, on purpose:

- everything the game server or you read directly: each server's `server.cfg`, `secrets.cfg`, txAdmin's files, logs, the downloaded
  tools and caches (`update-check.json`, `forks-cache.json`, `org-cache.json`: they can be thrown away);
- everything inside a server folder: `kora-server.json` (read by tools outside Kora), `.kora/` (the install's plan state, the update
  schedule, alert settings, paid-asset records, the **update lock**, which must stay a lock file). They travel with the folder when
  you move or copy it, and are written by the privileged side as the server's own account;
- `%ProgramFiles%\Kora\owners.json` (Windows): it belongs to the privileged side.

## Who can read it

It holds your saved licence keys and the hashes of remote-access tokens, so it is as private as `secrets.json` was:

- the `store` folder and the file are readable by you only: `0700`/`0600` from the moment they are created on Linux (the `-wal` and
  `-shm` files SQLite makes next to it too); on Windows the folder's inherited permissions are removed and only your account is
  granted access (the same `icacls` rule `secrets.json` had), so everything SQLite creates in it inherits that;
- Kora never writes through a link: a link in place of the folder or the file is refused;
- `sudo kora ...` inside someone else's Kora folder hands every file it creates back to that folder's owner (`lchown`), so you can still open your own data afterwards.

Saved licence keys are **not encrypted** (as before): the file's privacy is the protection. The key values are in their own table
(`license_key_material`) behind one small trait (`kora_core::store::material::KeyMaterial`), so a later version can move just those
into the operating system's keychain.

## The first start of a new version

The first time 0.8 opens the folder, every old JSON file is imported **in one transaction**, the rows are read back and compared with
what was parsed, and only then are the old files **moved** (not deleted) to `store/before-sqlite/` with their names (and
`paused-installs/<job>.json`). Before any later change of the data file's format, and before an old file is imported over existing
data, a consistent copy is made in `store/backups/` (the newest ten are kept).

- A file that cannot be read is **never lost or replaced**. A damaged saved-keys, server-list, remote-access or nodes file stays exactly
  where it is, the part of Kora that needs it says so in plain words (for keys: "Kora cannot read your saved licence keys ... no key
  was lost"), everything else works, and fixing the file is enough: it is imported the next time Kora starts. A damaged profile,
  saved-answers or stopped-install file only saves typing: it is moved to `before-sqlite` and Kora carries on.
- Importing twice does nothing. If Kora was stopped between the import and the move, the files are just moved next time. Two programs
  starting at once import once.
- Old Kora 0.6.x files have the same shape as 0.7's, so both are read.
- A saved key whose name only differs by case from another (possible in a hand-edited file) is kept as "Name (2)": no key is dropped.

## Going back to an older Kora (downgrade)

An older Kora (0.7 or earlier) does not know the data file and would start with an empty server list. To go back **without losing what you changed
since upgrading**:

1. With the new Kora still installed run `kora store export-json`. It writes the old-format files (`servers.json`, `secrets.json`,
   `profile.json`, `last-install.json`, `panel.json`, `hub.json`, `paused-installs/`) into `store/export-json/<time>/` (or the folder
   you give with `--to`; the files hold your saved licence keys, so they are `0600`).
2. Install the older Kora.
3. Copy those files into the Kora folder (`~/.kora`, `%LOCALAPPDATA%\Kora`).

If you already went back without exporting, the files from before the upgrade are in `store/before-sqlite/`: copy them back (you lose
what changed since). Leave the `store` folder where it is: the older Kora ignores it.

If you later upgrade again, the old-format files that are in the Kora folder are imported again (a file that differs from what was
imported replaces that part of the store, after a backup), so what the older Kora changed comes with you.

A newer data file meeting an **older** Kora that does know it (a version between) is refused politely ("saved by a newer version of
Kora ... update Kora"), and nothing is changed.

## Backups of the data file

`kora store backup` makes a consistent copy (SQLite's own `VACUUM INTO`, safe while the panel runs) in `store/backups/`, or where
`--to` says. It holds your saved licence keys: keep it as private as the original. `kora store status` shows the version, what is in
the file and the backups. (Backups of servers and their databases are separate and not part of this.)

To restore one: stop Kora and the panel, move `store/kora.db` away, copy the backup to `store/kora.db` (delete `kora.db-wal` and `kora.db-shm` if they exist).

## Jobs

Jobs (updates, installs, deletes ...) are written down as they run: the job, its events (the newest 1000), and how it ended. The
Settings page lists them (also after a restart). A job that was running when Kora Panel was closed or the computer restarted is shown
as **stopped by a restart**, with what to do. An install that used a **saved** licence key can be carried on ("Carry on" on the install page);
a pasted key you did not save is kept in memory only, as before, so it must be pasted again. Secrets never enter a job record or its events.
Finished jobs are kept for 30 days (at most 300).

## The audit log

An append-only record of every change made through the panel (on this computer, or from another device with its sign-in name,
provider and role) and through `kora` commands: when, which operation, which server, who, and the result (`ok`, `failed`,
`refused`, or `started` for a slow job, followed by a second row when it ends). No secrets, no request contents. Administrators
see it in **Settings > Activity log**; on the command line `kora audit [--server S] [--actor A] [--result R] [--days N] [--json]`.
Entries are kept 365 days (at most 100 000). It is a record for the person who runs Kora, not tamper-proof evidence: anyone who can open
the (private) data file could change it.

## For developers

Code: `crates/kora-core/src/store/` (`mod.rs` open/migrate, `schema.rs` the numbered migrations, `privacy.rs`, `legacy.rs` the
import, `backup.rs`, `export.rs`, `jobs.rs`, `audit.rs`, `material.rs`). Rules: migrations only go forward and are never edited once
released (add a new one); every change is one `BEGIN IMMEDIATE` transaction; read-modify-write belongs inside it
(`Registry::update`, `RemoteConfig::update_at`). Tests: `crates/kora-core/tests/store_processes.rs` runs several processes at once and
kills one mid-write; `store/legacy/tests.rs` imports v0.7-shaped homes.
