# draft-tracker

A writing log kept in SQLite: a terminal interface for reading it, and a one-line command for adding to it.

You tell it what a manuscript currently stands at, and it works out the session from the difference. That is the whole idea. Counting new words scattered across chapters is tedious and error-prone; reading the word count at the bottom of the document is neither.

```sh
$ draft-tracker log fireside 17900 1 -n "a good morning"
  Project      Fireside 1
  Date         2026-10-03
  Hours        1.00
  Project WC   17,900
               logged so far 17,228
  Session WC   672
  Words/Hour   672
  Notes        a good morning

Save this entry? [y/N] y
Saved. Fireside 1 is now at 17,900 words.
```

## Is it for you

It is for someone who already keeps a writing log, probably in a spreadsheet, and has started wanting to ask questions of it. How many hours went into that novella. Whether this year is ahead of last. What a realistic words-per-hour actually is, as opposed to what it feels like on a good day.

It assumes you work in a terminal and are comfortable in vim. It tracks words, time, projects and notes, and nothing else: no goal nagging, no streak gamification, no sync, no account.

It will import years of existing spreadsheets, which is the only sensible way to start.

## The interface

`draft-tracker` with no arguments opens on the entry form. The two read-only figures recalculate as you type, so a mistyped total is visible before it is saved rather than after.

```
 Draft Tracker · Entry
┌ Session ───────────────────────────────────────────────┐┌ Projects (8) ────────────────┐
│ Date        2026-10-03                                 ││  Emotional Evidence          │
│ Project     Fireside 1  logged so far 17,228           ││› Fireside 1                  │
│ Hours       1                                          ││  Glowstones                  │
│▌Project WC  17900                                      ││  Honesty Protocol            │
│ Notes       —                                          ││  Kathy's Donuts              │
│                                                        ││  Sandwich Debacle            │
│  ──────── worked out for you ────────                  ││  SitR Marketing              │
│ Session WC  672                                        ││  Veterin' Corps              │
│ Words/Hour  672                                        ││                              │
└────────────────────────────────────────────────────────┘└──────────────────────────────┘
 NORMAL  Project WC  ·  <leader><CR> submit  ·  ? help  ·  q quit
```

The dashboard answers the questions a spreadsheet makes hard. `j` and `k` walk back through the years, `$` shows all time.

```
 Draft Tracker · Stats
┌ Lifetime ─────────────────────────────────┐┌ 2025 ─────────────────────────────────────┐
│ Words       466,665                       ││ Words       186,457                       │
│ Hours       593.50                        ││ Hours       220.50                        │
│ Completed   68                            ││ Completed   29                            │
│ Words/Hour  786                           ││ Words/Hour  846                           │
└───────────────────────────────────────────┘└───────────────────────────────────────────┘
┌ 2025 projects (1–3 of 35) ─────────────────────────────────────────────────────────────┐
│ Trial of … █████████████████████████████████████████████████████████████████    87,973 │
│ Flask Vol… ████████████▋                                                        17,095 │
│ Fireside 1 ███████████▍                                                         15,429 │
└────────────────────────────────────────────────────────────────────────────────────────┘
┌ 2025 by category ──────────────────────────────────────────────────────────────────────┐
│ █ SitR Book 66.4% (123,790)  █ SitR Short Story 15.7% (29,223)                         │
│ █ SitR Blog Post 12.0% (22,454)  █ Non-Fiction Essays 4.3% (8,090)                     │
│ █ Kids Book 0.8% (1,578)  █ SitR Marketing 0.7% (1,322)                                │
│████████████████████████████████████████████████████████████████████████████████████████│
│████████████████████████████████████████████████████████████████████████████████████████│
└────────────────────────────────────────────────────────────────────────────────────────┘
┌ Year over year ────────────────────────────────────────────────────────────────────────┐
│ 2023 █████████▉                                                                34,261  │
│ 2024 ██████████████████████████████████████████████████████████████████████   244,148  │
│ 2025 █████████████████████████████████████████████████████▌                   186,457  │
│ 2026 ▌                                                                          1,799  │
└────────────────────────────────────────────────────────────────────────────────────────┘
 by year  ·  j k year  ·  h l scroll  ·  0 now  ·  $ all time  ·  ? help  ·  q quit
```

Five screens: the entry form, the entry history, projects, categories, and the dashboard.

History lists every entry and lets you correct one, delete it, or move it to the project you meant. Projects shows start and end dates, totals, days written, hours per day and words per hour, and cycles a project between open, completed and abandoned on one key. Categories manages categories and the types above them.

## Keys

Vim-flavoured, and none of them uses a control key, so the same keymap works on every platform. `<leader>` is the space bar; holding it shows a menu of what it can be followed by. `?` lists every key for the screen you are on.

| Key | |
| --- | --- |
| `<leader>e` `<leader>h` `<leader>p` `<leader>c` `<leader>s` | entry, history, projects, categories, stats |
| `j` `k` | move; inside a form, between fields |
| `h` `l` | step a choice: a project, a status, a category, a type |
| `i` `a`, `Esc` | start and stop typing; on a choice field, narrow its list |
| `/` | filter a list |
| `A` | widen a list from open projects to all of them |
| `o` | create a project, category or type |
| `x` | cycle a project's status |
| `dd` | delete, then `y` to confirm |
| `<leader><CR>` | submit or save |
| `[` `]` `0` `$` | on the dashboard: older year, newer year, this year, all time |
| `q` | quit |

Colours follow [melange](https://github.com/savq/melange-nvim) and assume a terminal with true colour.

## Installing

Needs a Rust toolchain. SQLite is compiled in, so there is nothing else to install.

```sh
git clone https://github.com/nebulous-code/draft_tracker
cd draft_tracker
cargo install --path .
```

That puts `draft-tracker` on your path. An alias is worth having for something typed daily:

```sh
alias dt='draft-tracker'
```

## Logging a session

```sh
draft-tracker log <project> <words> <hours> [options]
```

The project is matched on any part of its title, case insensitive, against the open projects, so `fire` finds `Fireside 1`. An exact title wins outright, so a project whose name sits inside another's is still reachable. A fragment matching several is refused with the candidates listed; one matching only a finished project says so rather than claiming the project does not exist.

The word count is the project's new running total. On a project's first entry there is no earlier total to subtract from, so the number is taken as the session itself, and the summary says so.

| Option | |
| --- | --- |
| `-d`, `--date <when>` | `today` (the default), `yesterday`, or a date |
| `-n`, `--notes <text>` | a note for the session |
| `-y`, `--yes` | save without asking |
| `--any` | allow a completed or abandoned project |
| `--db <path>` | use a different database |
| `--config <path>` | use a different configuration file |

`draft-tracker --version` reports the version, the commit it was built from and whether that build included uncommitted changes, which is the quickest way to tell whether an installed copy is the build you think it is.

Nothing is written until the entry has been shown and agreed to. The answer is read from whatever standard input is, so piping one in works; `--yes` is for a script.

A session has a floor of 0.25 hours and zero is refused. This is an accounting rule rather than an input guard: a minute spent counts as fifteen, because a session rounded down to no time makes a words-per-hour figure either undefined or absurd.

A session may be negative. A revision day that cuts words is a real working day, and the figures are built to say so rather than to flatter.

## Importing what you already have

```sh
draft-tracker import ~/writing-journals --dry-run   # report, write nothing
draft-tracker import ~/writing-journals
```

Every `.ods` and `.csv` in the directory is read, and a workbook's `Data` sheet is used. Columns are located by header name, trimmed and matched without regard to case, taking the leftmost match, so a sheet carrying its own extra columns still imports and a header reading `  Day WC` does not break it.

An entries file needs `Date`, `Project`, `Hours` and `Category`, plus at least one of `Day WC` and `Project WC`. `Type` and `Notes` are optional. Where `Day WC` is missing, the session is the rise in `Project WC`, which is the arithmetic a spreadsheet usually encodes as `=I3-I2`.

A projects file carries `Project` and `Status` but no `Date`, and sets each project to `open`, `completed` or `abandoned`.

The rules are deliberately forgiving about the state real spreadsheets are in. A row recording neither words nor time is skipped, which retires missed-day bookkeeping without having to recognise whatever you called it. A project's category is the majority of its rows' categories, and a category's type the majority of the rows carrying it, so one mistyped cell cannot move a project. Repeated whitespace inside a name is collapsed, because a category entered as `SitR  Book` would otherwise become a second category with nothing in it. Everything the import changed or could not account for is reported, and it refuses to run against a database that already holds data unless given `--reset`.

## Where your writing log lives

One SQLite file, at `$XDG_DATA_HOME/draft_tracker/draft_tracker.db`, or `~/.local/share/draft_tracker/draft_tracker.db`. `--db <path>` and the `DRAFT_TRACKER_DB` variable override it.

It is a plain database with four tables and no bespoke format, so anything that reads SQLite can read your writing history. Nothing is locked up.

It runs in WAL mode, so a `-wal` and a `-shm` file sit beside it while the program is open and are folded back in when the last connection closes. One consequence worth knowing: **copying only the `.db` file while the program is running will miss recent entries**, because they are still in the write-ahead log. Use the backup command rather than `cp`.

`draft-tracker status` prints the totals and your open projects without opening the interface.

## Backups

A backup is taken automatically when the interface opens and whenever an entry is saved, and `import --reset` refuses to run until one succeeds, because that command deletes every row.

Backups are **skipped when nothing has changed**. A fresh copy is compared against the most recent one, so opening the program to read does not consume a generation. That is what makes `backups_to_keep` meaningful: twenty backups are the last twenty states your log was actually in, not the last twenty times you started the program.

Set `backups = false`, or pass `--no-backup` for one run, to turn the automatic ones off; asking for one outright still works.

A backup never blocks you. If the directory is missing or the disk is full you get a warning and your entry is still saved, because losing the words to protect a copy of them would be the wrong way round.

```sh
draft-tracker backup                 # into the configured directory
draft-tracker backup ~/archive.db    # exactly there, and never overwriting
```

Either way it opens what it wrote and reports an integrity check with the entry, word and project counts, so you can see the copy contains your writing rather than merely existing.

Backups land beside the database by default. That protects you against mistakes and not against losing the disk, so point `backup_directory` at something that leaves the machine. Snapshots or replication on that directory pair well with this: the program keeps the recent states, and the filesystem keeps the long tail.

## Configuration

Optional. `~/.config/draft_tracker/config.toml`, or wherever `--config` points. `draft-tracker config --init` writes a commented file with every default filled in.

```toml
database = "/tank/writing/draft_tracker/data/draft_tracker.db"
backup_directory = "/tank/writing/draft_tracker/backups"
backups_to_keep = 20
backups = true
name = "draft_tracker"
```

`name` prefixes backup filenames and defaults to the database's filename, so one directory can hold backups of several logs and a scratch database can never prune the real one's.

A command line option beats an environment variable (`DRAFT_TRACKER_DB`, `DRAFT_TRACKER_BACKUP_DIR`), which beats the file, which beats the default. `draft-tracker config` prints what is in force and **which of those decided it**, which is the quickest way to be certain which log you are about to write to.

```
database           /tank/writing/draft_tracker/data/draft_tracker.db   config file
backup_directory   /tank/writing/draft_tracker/backups                config file
backups_to_keep    20                                                 default
name               draft_tracker                                      default
```

## How it is built

Rust, with [ratatui](https://ratatui.rs) over crossterm and [rusqlite](https://github.com/rusqlite/rusqlite) with SQLite bundled in. No async runtime: nothing here waits on a network.

Two decisions shape the schema. A session's word count is stored, and a project's total is the sum of its sessions, so correcting one entry never means rewriting a chain of running totals. And a project's status is `open`, `completed` or `abandoned` rather than a boolean, because a piece written and never shipped is not finished work and counting it as such makes the figures lie.

Reading the code: `src/db` is the data layer, `src/import` reads spreadsheets, `src/tui` is the interface, `src/cli.rs` is the `log` command. The design is written up in `docs/mvp_design/technical_design.md`, which records why things are the way they are rather than only what they do.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

Tests run against in-memory databases and render screens into a buffer, so none of them touches a real log.

## License

MIT. See `LICENSE`.
