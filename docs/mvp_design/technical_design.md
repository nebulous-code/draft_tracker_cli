# Draft Tracker - Technical Design

Companion to `mvp_design_doc.md`, which holds the requirements. This file records how the MVP is built and why each choice was made. Every number quoted here was measured by parsing the three `.ods` files, not estimated.

## Stack

Rust, with `ratatui` over `crossterm` for the TUI and `rusqlite` (with the `bundled` feature) for storage. Bundling SQLite means the binary has no system library dependency, which matters because this is a personal tool that should keep working after an OS upgrade. Nothing here needs async, so there is no runtime.

The importer reads `.ods` with `calamine`, which handles the OpenDocument repeated-row and repeated-column encodings that these files use heavily. That encoding is the one real risk in the import, so validating `calamine` against the actual files is the first task in the build sequence rather than an assumption baked into later work. The fallback, if it mishandles them, is a direct `zip` + `quick-xml` read of `content.xml`, which is already proven out: the expected-import figures below came from exactly that approach.

The importer ships as a subcommand, `draft-tracker import <dir>`, not a standalone script. It needs the same schema and the same project-and-category resolution logic as the app, and a subcommand shares that code instead of duplicating it in a second language that could drift. It is reusable by any writer whose journal meets the input contract below, and contains no project-specific rules.

Database path is `$XDG_DATA_HOME/draft_tracker/draft_tracker.db`, falling back to `~/.local/share/draft_tracker/draft_tracker.db`, overridable with `--db <path>` or `DRAFT_TRACKER_DB`. The override exists so the import can be rehearsed against a scratch copy before touching the real database.

Schema versioning uses SQLite's own `user_version` pragma with hand-written stepwise migrations. A migration framework would be more machinery than a four-table schema earns.

## Data model

Types are the parent and categories hang off them, so a project carries one category and its type is derived through that. This is the business distinction: the category is what the thing is (short story, novel, blog post, kids book) and the type is what register it is written in (fiction, non-fiction, creative non-fiction).

```sql
PRAGMA foreign_keys = ON;

CREATE TABLE types (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  description TEXT
);

CREATE TABLE categories (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  type_id     INTEGER NOT NULL REFERENCES types(id),
  description TEXT
);

CREATE TABLE projects (
  id                  INTEGER PRIMARY KEY,
  title               TEXT NOT NULL UNIQUE,
  category_id         INTEGER REFERENCES categories(id),
  status              TEXT NOT NULL DEFAULT 'open'
                        CHECK (status IN ('open', 'completed', 'abandoned')),
  description         TEXT,
  created_at          TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE entries (
  id                 INTEGER PRIMARY KEY,
  entry_date         TEXT    NOT NULL,              -- ISO-8601, 'YYYY-MM-DD'
  project_id         INTEGER NOT NULL REFERENCES projects(id),
  session_word_count INTEGER NOT NULL,              -- may be negative; revision cuts are real
  hours              REAL    NOT NULL CHECK (hours >= 0.25),
  notes              TEXT,
  created_at         TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_entries_project ON entries(project_id);
CREATE INDEX idx_entries_date    ON entries(entry_date);
```

`status` replaces an earlier `is_completed` boolean because finishing a project and giving up on one are different outcomes that a boolean collapses. A piece written and never shipped is abandoned, not completed, and counting it as either makes the completed-projects figures lie. A `CHECK` constraint rather than a lookup table, because three values fixed by their meaning are not data the writer needs to extend, and storing the word keeps ad hoc SQL against this database readable.

`projects.title` is unique because the importer keys projects by name to stitch them across the three yearly files; three projects genuinely span two files, so without that key `Flask Vol 1` would import twice.

`category_id` is nullable, since the requirements mark only Title as required. All 77 imported projects get a category, so a null only arises on a project created by hand without one. Statistics bucket nulls as `Uncategorized` rather than dropping them, so the category distribution bar always sums to 100 percent of the year's words.

The entry stores only the session word count, never the Project WC that was typed to produce it. Storing both would create two sources of truth that fall out of sync the moment a past entry is edited, and would force a cascading rewrite of every later row's Project WC. Storing just the session count makes edits local: change one row and every derived total is still correct, because totals are order-independent sums.

There is deliberately no stored baseline for work done before tracking began. An earlier draft of this design carried `starting_word_count` and `starting_hours` on the project, because three projects had been tracked from partway through a manuscript. Importing the 2023 journal replaced all three with real dated entries, which is strictly better: the words and the hours now belong to the days they happened on, so they count toward a year as well as a project, and nothing has to be kept in sync by hand. With every baseline at zero the columns earned nothing, so they are gone.

The cost is that a project begun mid-manuscript has nowhere to record what came before. Two things cover it. On import, a project whose earliest entry implies pre-tracking words is reported by name with the number, so the gap is visible rather than silently dropped, and can be closed by importing the earlier journal as its own file. In the entry form, the first entry on a project asks for the words written that session rather than the manuscript total, since there is no prior total to subtract from; every later entry takes the total as usual.

## Derived metrics

Every project metric is a plain aggregate of that project's entries, which is what keeps the project screen and the dashboard in agreement.

`abandoned` and `completed` are treated identically by every derived calculation. Both mean the writer has stopped, so both get an End Date from the last entry, and both are held out of the entry form's default project list. The distinction exists for one reason only, which is that abandoned work must not be counted as finished: it is excluded from the Completed Projects figures, and its bar is coloured differently. Anywhere else, a rule that applies to a completed project applies to an abandoned one.

| Metric | Definition |
| --- | --- |
| Start Date | `MIN(entry_date)` for the project |
| End Date | `MAX(entry_date)` when `status` is not `open`, otherwise null |
| Total Word Count | `SUM(session_word_count)` |
| Days Written | `COUNT(DISTINCT entry_date)` |
| Hours Written | `SUM(hours)` |
| Hours per Day | Hours Written / Days Written, null when Days Written is 0 |
| Words per Hour | Total Word Count / Hours Written, null when Hours Written is 0 |

Because every word and hour now belongs to a dated entry, Dashboard Lifetime Words equals the sum of the project totals exactly, and the same holds for hours. Nothing has to be reconciled between the two screens, and a figure that does not add up is a bug rather than an expected artefact.

Dashboard queries all take an optional year and resolve lifetime when it is absent, so a single query serves both sections and the year is never hardcoded.

| KPI | Definition |
| --- | --- |
| Words | `SUM(session_word_count)`, filtered to the year when given |
| Hours | `SUM(hours)`, filtered to the year when given |
| Completed Projects | lifetime: count where `status` is `completed`; year: that, and `MAX(entry_date)` falls in the year |
| Projects graph | per project with an entry in the year, its summed words; title truncated to 10 characters; bar green when completed, muted red when abandoned, default when open |
| Year over Year | words grouped by `strftime('%Y', entry_date)` |
| Category distribution | per category, its year words over the year total, drawn as one segmented bar |

Words per hour is allowed to go negative, since a revision day that cuts words is a real working day. It is undefined rather than negative when hours is 0, which the two historical rows below require; those render as an em dash.

## Import

This project is intended to go public as a portfolio piece, so the importer carries no knowledge of any particular writer's projects, categories or titles. Everything it does is a rule that applies to any writing journal in the expected shape. Personal data cleanup happens in the spreadsheets instead, which is where it belongs: the sheets are the writer's own records, they are gitignored, and a clone of this repo should not make anyone read through someone else's project names to understand the code.

### Input contract

The importer reads every `.ods` and `.csv` in the given directory, taking the `Data` sheet of a workbook and the whole file of a CSV, and locates columns by header name, trimmed and matched without regard to case, taking the leftmost match for each. Leniency here costs nothing and spares a writer a failed import over a header reading `date` or `Day WC ` with a trailing space, which these very sheets contain. Supporting CSV matters for a tool meant to be usable by someone else: a spreadsheet of this shape is a convenience, not a requirement, and a plain CSV is something a writer can produce from any journal, by hand, without owning LibreOffice.

Two kinds of file are recognised. An entries file is one whose headers include `Date`. A projects file is one carrying `Project` and `Status` but no `Date`, and it supplies the per-project metadata that a log of daily sessions cannot express. Detection is by header rather than by filename, so neither file has to be named a particular way.

For an entries file, required headers are `Date`, `Project`, `Hours` and `Category`, plus at least one of `Day WC` and `Project WC`. Optional headers are `Type` and `Notes`, and whichever of the two word-count columns is not supplied. A missing required header aborts the import with a message naming it, rather than silently importing a column of nothing. Any other column is ignored, which is what makes the leftmost-match rule safe: these sheets repeat `Date`, `Day WC` and `Hours` further right in a derived rolling-average block, and the first occurrence of each is always the real one.

Locating by name rather than position is what lets the derived block be ignored without hardcoding where it starts. It also means a writer can add columns of their own without breaking the import.

`Type` is optional because a type belongs to a category, not to a row, so stating it once anywhere in the data is enough; blanks are ignored by the vote that resolves it. A category that is never typed anywhere aborts the import naming that category, so the data cannot produce a typeless category.

A projects file requires `Project` and `Status`, where the status is `open`, `completed` or `abandoned`. It may also carry `Description`. A status is trimmed of surrounding whitespace and matched without regard to case, since the file is edited by hand and `Abandoned ` means what it says; an unrecognised value aborts the import naming the row, rather than guessing. Two rows for one project stop the import rather than one silently overriding the other, since there is no way to tell which the writer meant. A project named there but absent from every entries file is reported and skipped, since a project with no sessions has nothing to track; a project in the entries with no row in the projects file defaults to `open`. Every other column is ignored, which lets the file carry word counts, dates and categories as decision aids while it is being filled in without those becoming a second source of truth.

`Category` is required as a header but may be blank on any individual row, for the same reason: a category belongs to the project, so stating it once is enough and the vote ignores blanks. This is what lets a supplementary file carry only the columns that change, naming projects that other files already categorise. A project with no category anywhere imports with a null `category_id` and is reported, appearing under `Uncategorized` in statistics rather than being dropped.

A leading byte-order mark is stripped before headers are matched. Spreadsheet applications write one when saving UTF-8 CSV, and without stripping it the first header silently fails to match.

A workbook supplies real date values. A CSV is parsed as ISO-8601 `YYYY-MM-DD` first, then as the month-first `MM/DD/YY` and `MM/DD/YYYY` that spreadsheet applications produce from a US locale. Month-first is genuinely ambiguous against day-first for the first twelve days of a month, so the importer reports the parsed date range for each file and the reader is expected to check it; ISO-8601 is the documented preference for anything typed by hand.

### Rules

Rows are skipped when they have no date, when `Day WC` and `Hours` are both zero or blank, or when `Project` is empty. The zero-work rule is what retires missed-day bookkeeping generically: a row recording no words and no time is not a session, whatever it is called, so no sentinel project name has to be recognised. A row with hours but no words is kept, because time spent without net new words is real work.

A kept row recording less than the 0.25 hour minimum, including a session with words but no time at all, is raised to 0.25 and reported. Without this the schema's floor would abort the import of any journal that recorded a session as taking no time, which is a common way to log a drive-by edit. Raising rather than rejecting applies the same accounting rule the entry form enforces, and reporting it keeps the change visible instead of silently rewriting what the writer recorded. Two rows in this data were raised by hand in the spreadsheets before the rule existed, so the importer finds nothing left to raise.

Either word-count column can drive an entry, and a writer will usually have only one of them to hand. `Day WC` is used directly when present. When it is blank, the session count is derived as the difference between this row's `Project WC` and the previous entry's `Project WC` for the same project in date order, which is the same arithmetic the spreadsheets encode as `=I3-I2` and the same model the entry form uses. Supporting the derivation matters because knowing a manuscript's running total is easier than counting new words across chapters, which is the premise the whole application rests on; requiring `Day WC` would mean doing that subtraction by hand before the import could read it.

`Day WC` wins wherever both are present. Some projects have internal drift between their summed day counts and their `Project WC` series, because a Project WC can be reset mid-draft when a manuscript is restarted, so the two columns cannot both be honoured. The day counts are the record of work done, which is what the requirements ask to store.

A name is trimmed and runs of whitespace inside it are collapsed to single spaces, with every change reported. A project or category differing from another only by a repeated space is a typo every time, and left alone it silently becomes a second record: a project split in two, or a phantom category with no projects in it. This is easy to introduce and invisible while editing, because OpenDocument stores a second consecutive space as a `<text:s/>` element rather than as a space character.

A project is identified by its title and matched across files, so a project spanning several years becomes one record. Its category is the majority vote of its rows' `Category` values, ignoring blanks, and each category's type is the majority vote of the `Type` values on rows carrying it. Voting rather than last-write-wins means a single mistyped row cannot move a project, and it degrades gracefully on a sheet that was never cleaned up.

Nothing is stored for work predating the data. Where a project's earliest entry has a `Project WC` exceeding its `Day WC`, the difference is words written before tracking began; the importer reports each such project and the number rather than inventing an entry or a stored baseline, because the honest fix is to import the earlier journal. No spreadsheet column records pre-tracking hours at all, so a words-only baseline would have skewed every rate metric upward anyway, which is the second reason not to keep one.

Project status comes from the projects file when one is present, and defaults to `open` otherwise. Nothing in a log of daily entries records whether a project was finished, and inferring it from staleness cannot tell a completed piece from an abandoned one, which is exactly the distinction that matters. An abandoned project keeps an end date, since the writer stopped on a known day, but is excluded from the completed count.

The importer refuses to run against a non-empty database unless given `--reset`, so it cannot silently double-import, and it prints a report of what it created and skipped.

### Preparing the sheets

The three spreadsheets were normalized once to meet the contract above, by a script kept in the gitignored `historic_trackers/` directory alongside the data it describes. A fourth file, a CSV transposed from a 2023 paper journal, was added later and needed no normalizing beyond two corrections: a mistyped year that placed one row in 2026, and nineteen rows for early 2024 that had been transposed a second time and already existed in the 2024 workbook. It renamed the second `Project` header to `Category` so every needed column has a unique name, merged projects that had been tracked under two names, consolidated several one-day pieces into the projects they belong to, applied category renames, rewrote the `Type` column from each row's category so spelling and mislabelled rows were fixed in one pass, filled in three rows that had lost their project name, and made one merged project's `Project WC` column cumulative so the sheet stayed self-consistent.

One finding from that work is worth recording, because it would be easy to repeat. These sheets store formulas as strings beginning `of:=`, where the `of` prefix lives inside an attribute value rather than an element name. A naive XML round-trip emits namespace declarations only for prefixes it sees on element and attribute names, so it drops `xmlns:of` and every formula in the workbook becomes `Err:510`. The normalizer splices the original root start tag back into its output and refuses to write a file that has lost a declaration. Any future tool that rewrites these files needs the same care.

A second finding came from the same corner of the format and is worth the same warning. OpenDocument writes a run of consecutive spaces as a `<text:s/>` element, so reading a cell by concatenating its text nodes drops them: a category entered as `SitR  Book` reads back as `SitR Book`, and a typo disappears. The analysis scripts used to plan this work did exactly that and reported one category fewer than the data held; the importer, which renders `<text:s/>` faithfully, found the extra one on its first run. Reading a cell means rendering it as a spreadsheet would, not concatenating its text.

After normalization every category's type is decided unanimously, so the majority vote never has to break a tie on this data. It stays in the importer because another writer's sheets will not be as tidy.

### Verification

The normalized sheets were checked three ways before being installed: the derived `Stats` sheet converts byte-identically to the originals with zero formula errors, a field-level diff of the `Data` sheet shows changes confined to the `Project`, `Type`, `Category` and one project's `Project WC` columns with `Date`, `Day WC`, `Hours` and `Notes` untouched, and a simulated import reproduces the same word and hour totals as the untouched originals. Totals being unchanged is the point: the normalization moved labels, never work.

### Expected import

| | Entries | Words | Hours |
| --- | --- | --- | --- |
| 2023 | 124 | 34,261 | 56.00 |
| 2024 | 320 | 244,148 | 314.00 |
| 2025 | 207 | 186,457 | 220.50 |
| 2026 | 3 | 1,799 | 3.00 |
| Total | 654 | 466,665 | 593.50 |

78 projects, 10 categories, 3 types. 239 rows skipped as no work logged, which covers both missed-day bookkeeping and blank future-dated rows. No project reports pre-tracking words, so every word and hour in the data belongs to a dated entry.

| Category | Type | Projects | Words |
| --- | --- | --- | --- |
| SitR Book | Fiction | 9 | 265,419 |
| Short Story | Fiction | 14 | 71,968 |
| SitR Blog Post | Creative NF | 30 | 34,618 |
| Other Book | Fiction | 2 | 33,355 |
| SitR Short Story | Fiction | 8 | 33,176 |
| Non-Fiction Book | Non-Fiction | 1 | 9,607 |
| Non-Fiction Essays | Non-Fiction | 4 | 9,159 |
| Kids Book | Fiction | 5 | 5,522 |
| SitR Marketing | Non-Fiction | 1 | 2,570 |
| Flash | Fiction | 4 | 1,271 |

Every category's type resolves unanimously, so the majority vote never has to break a tie on this data. It stays in the importer because another writer's files will not be as tidy.

Oddities carried through intact rather than cleaned, because they are real working days: five entries with a negative day count, and 26 entries recording time with no net words. Those 26 are a December streak on one project totalling 6.50 hours, and they are the reason words per hour has to tolerate a zero numerator as well as a zero denominator.

85 days carry more than one entry, at most 4, and two days carry two entries for the same project, which the entry form supports.

## Settings and backups

Settings come from a file at `~/.config/draft_tracker/config.toml`, resolved behind a command line option and an environment variable and in front of the built-in defaults. Each resolved value carries the source that decided it, which is what lets `draft-tracker config` print not just what is in force but why. With a live log and a scratch one in play that is the difference between knowing which database is about to be written to and assuming it.

Automatic backups can be turned off, by `backups = false` in the file or `--no-backup` for one run. Asking for a backup outright still takes one: somebody who typed the command meant it. The setting exists because a writer whose filesystem already snapshots the live database may reasonably not want a second mechanism copying it.

Unknown keys in the file are refused rather than ignored. The file is edited by hand, and a `backups_to_keeep` that silently does nothing is worse than an error naming it.

A backup is `VACUUM INTO`. One statement, it reads the logical database so the write-ahead log is not a consideration, it compacts as it goes, and it refuses to write over an existing file, which is exactly what is wanted for a named destination: a mistyped path cannot quietly replace an archive.

Backups are skipped when nothing has changed. A fresh copy goes to a temporary file and is compared against the most recent backup byte for byte; matching means the temporary file is discarded. This is exact rather than a heuristic, because `VACUUM INTO` of unchanged content is byte-identical, which was measured before the design depended on it. The reason it matters is retention: without it, `backups_to_keep` counts program launches, so an afternoon of opening the dashboard to read evicts a week of history. With it, the count measures states the log was actually in, which is the thing a writer would mean.

Byte comparison rather than a hash: both files are already on disk, the comparison is exact with nothing to reason about regarding collisions, and it saves a dependency.

Automatic backups are taken when the interface opens and after every saved entry. The first is the more useful of the two, because it captures the state before a session can go wrong; the second is what stops a day's writing sitting unprotected until the next launch. Neither may block: a failure is reported and the work carries on, because losing words to protect a copy of them is the wrong way round. `import --reset` is the exception, and refuses to proceed until a backup succeeds, since it deletes every row and a flag is thin protection for that.

Two bugs here were found by running it rather than by reading it, and both are worth recording because both were silent.

Verifying a backup opened it the ordinary way, which set its journal mode and so rewrote the file being checked. That made it stop matching a fresh copy, so every later backup was written as though something had changed, and the skip logic never fired once. Backups are now opened read-only and without migrations.

Ordering was by filename, on the reasoning that a timestamp sorts lexically. It does, until two backups land in the same second and a counter is appended: `-2` sorts before `.`, so the newest looked like the oldest, and pruning deleted the wrong file. Names now carry milliseconds, and ordering is by modification time with the name breaking ties, because time is what is actually meant. Encoding an ordering into names and then deleting files by it put the newest backup at risk to save a `stat` call.

## A second session

The command line writes entries, so two connections against one database is the ordinary case rather than an edge. The database copes: WAL allows a writer alongside readers, there is a five second busy timeout, and a write from the command line lands while an interface is open. That was checked against a held-open connection rather than assumed.

What does not cope on its own is the interface's cached figures. A session count is the difference between what the writer typed and what the project already holds, so a total that went stale while the screen was open inflates the next entry without complaint: with 17,228 cached and 17,900 actually logged, typing the true 18,000 computes 772 where the answer is 100. The entry is wrong, nothing reports it, and the writer finds out weeks later looking at a day that reads oddly.

`PRAGMA data_version` answers exactly the needed question. SQLite leaves it alone for the connection's own writes and changes it when another connection commits, so comparing it against a remembered value asks "has somebody else changed this since I last looked" and nothing else. That it ignores one's own writes was verified, because the whole design rests on it: were it otherwise, every second entry of the day would be refused for no reason.

It is checked at two moments rather than continuously: changing screen, and submitting an entry. Those are the points at which the cached figures are about to be read or written against, and polling between them would buy nothing that checking at the point of use does not.

On a screen change the data is reloaded and the writer told. On a submit the data is reloaded and the submit refused once, which matters more than it looks: the recalculated session goes on screen so that the number written is a number the writer has seen. Silently correcting 772 to 100 would be right about the arithmetic and wrong about consent, and the one thing worse than a figure the writer did not expect is a different figure they never saw.

## Logging from the command line

The interface is for reviewing, fixing and planning. Logging a day's work is one short command, and a writer with this binary on their path should not have to open a screen to do the thing they do every day:

```
draft-tracker log fireside 17900 1 -n "a good morning"
```

The project is matched on any part of its title, without regard to case, against the open projects. An exact title wins outright, so a project whose name is contained in another's is still reachable: without that rule `Flask` could never be named while `Flask Vol 1` existed. A fragment matching several projects is refused with the candidates listed, because guessing which project a day's work belongs to is not a thing to do quietly. A fragment matching nothing lists the open projects, and a fragment matching only a finished one says so rather than claiming the project does not exist, since that is the more useful answer; `--any` logs against it anyway.

The word count is the project's new running total, and the session is derived from it exactly as on the entry form, including taking the number as the session itself on a project's first entry. The hours floor applies, and is checked before the project is resolved so that a message about hours is not buried behind one about a name.

Nothing is written until the entry has been shown and agreed to. The summary carries the reference total as well as the session, so a surprising figure can be reasoned about before it is saved rather than after. There is no editing at the prompt: the answer is yes or no, and running the command again costs nothing.

The answer is read from whatever stdin is, so piping one in works for a script. End of input is treated as neither yes nor no but as not having been asked, and refuses with a pointer to `--yes`: a writer who answered nothing has not declined. `--yes` saves without asking, for a cron job or an alias.

`status` lists the open projects and their totals, because logging from the command line needs their names and a dashboard cannot be read from a pipe.

## Screens

The app opens on the entry form, which is the daily path.

**Entry** takes a date defaulting to today and accepting past or future, a project stepped from the open list with a key to widen it to completed and abandoned projects too, since work does get picked back up, hours as a decimal of at least 0.25, and the project's current total word count. Session word count and words per hour are read-only and recalculate as you type. The session count is the typed total minus the sum of the project's existing entries. That reference is deliberately not date-ordered: the number you type is the manuscript's count right now, so it is measured against everything logged, which also means back-dating an entry needs no recalculation of anything. On a project's very first entry there is no prior total, so the form asks for the session count directly and labels it as such, which is how a manuscript started before tracking gets onto the books. A negative session count renders in the warning colour with the words cut spelled out, and submits normally.

The project is a choice field, stepped with `h` and `l` exactly as a project's category and status are on the projects screen, and drawn in the same brackets so it reads as something stepped rather than typed. `j` and `k` walk between fields like `Tab`, which is the correction to a first attempt where they moved the project list from wherever the cursor happened to be: a key that reaches past the focused field to change something else is a key that will eventually change the wrong thing. `i` on the project field starts filtering rather than refusing, because a picker holding dozens of projects is narrowed far faster by typing than by stepping.

Hours has a floor of 0.25 and zero is refused: submitting with less is blocked, with the reason shown rather than the keystroke silently doing nothing. This is a deliberate accounting rule, not a guard against bad input. A minute spent is counted as fifteen for the purpose of a words-per-hour figure, because a session rounded down to nothing produces an undefined or absurd rate and misrepresents the day. Encoding it means the figure can never be distorted by a session that claims no time. All 654 imported entries already satisfy it, and every recorded value is a multiple of a quarter hour.

**Entry history** lists every entry, newest first, with a project filter, and edits or deletes one. This is what makes a mistyped 20,000 recoverable, which is the case the requirements call out.

The number edited here is the session count itself, not a manuscript total. The entry form derives a session from a running total, but once a row is stored there is no unambiguous total to re-derive it from, since later entries have moved the running figure on. Editing the figure the row actually holds is the only honest option, and the panel labels it so, because the same words mean something different on the two screens.

An entry's project is editable, stepped with `h` and `l` exactly as every other choice field in the application, with a picker listing the candidates beside the panel. The picker appears only while the project is the field in hand, because it is the one field that needs a list and the panel has other rows to show the rest of the time. It offers closed projects as well as open ones, since the reason to move an entry is that it was filed somewhere wrong and the right home may well be finished. Typing narrows the list and lands on what it finds, rather than leaving the old project pending behind a list that no longer contains it; nothing is written until the edit is saved. An earlier draft of this design left the project read-only on the grounds that a second picker was too much machinery and that deleting and re-adding the row would do. That reasoning expired once choice fields existed elsewhere, and it had always understated the cost: re-adding loses the notes and makes the writer work out what to type against a running total.

The whole table is loaded rather than paged. A limit would silently put the oldest entries out of reach on the one screen whose job is fixing mistakes, and a writing life's worth of sessions is a few tens of thousands of rows.

An edit left open is closed when the screen is left, with a message saying the change was discarded. Keeping it open would mean returning later to a panel holding changes with no memory of having started them, and quitting with an open edit asks for confirmation the same way an unsubmitted entry does.

**Projects** opens on the open projects alone, with `A` widening it to all of them, because a writing life accumulates finished work and the handful still going is what a writer is usually looking at. The title states which view it is, so a short list is never mistaken for missing data. It lists each project with the derived metrics above, creates and edits them, and cycles status.

Cycling the status of a project in the open-only view holds that row in view until the cursor moves off it. Letting it vanish the instant it stopped being open would hide what the keystroke did and strand the cycle halfway, with no way to carry on to abandoned. The cursor moves where it was sent and the row falls away behind it. Editing covers title, category, status and description. Status cycles on a single key with no panel opening, which is what makes a review pass over 78 projects quick; the list shows the compact figures and a pane beneath it gives the selected project's full set.

Deleting a project is refused outright when it has entries, rather than offered behind a confirmation. `entries` cascades on delete, so agreeing to the prompt would destroy the record of the work done, and abandoning the project is what the writer almost always means instead. The refusal names the entry count and points at the status key. The repository enforces this too, independently of what the screen believes, so a stale list cannot destroy work.

**Categories** shows categories and types side by side, in two panes that `Tab` moves between. Both can be created, edited and deleted. They sit on one screen because a type is a category's parent, and editing one while unable to see the other makes the hierarchy hard to reason about; three types did not justify a screen of their own.

Deleting is refused while anything still points at the record, naming how many projects use a category or how many categories use a type. A category with no type cannot be saved, since the schema requires one, and the message says to create a type first rather than only refusing.

A field whose value comes from a fixed set, a project's category and status and a category's type, steps with `h` and `l` rather than being typed. Pressing `i` on such a field says so instead of appearing to do nothing.

**Stats** is the dashboard, with Lifetime and the selected year as titled sections. The year selector is wired up in the MVP rather than deferred: the queries are year-parameterised anyway, so the only additional work is two keybindings, which is less effort than leaving a hardcoded year to remove later. Each section also carries a words-per-hour figure, since it costs nothing to divide two numbers already on screen.

All time sits at the end of the sequence of years, so the selection walks 2023, 2024, 2025, 2026, all time. `j` and `k` move through it, which is the pairing the eye expects when the years are drawn stacked vertically; `[` and `]` do the same. `$` jumps to the end of the sequence and `0` returns to the year the writer is living in, borrowing both meanings from a line of text. Stepping moves between years that have entries rather than through the calendar, so a keystroke never lands on a year of zeroes; the current year is always offered even before anything is logged in it. Either end says so rather than sitting mute.

In the all-time view the selected scope is lifetime, so the two KPI panels would print the same four numbers side by side. One panel spans the row instead, which also makes the mode change visible at a glance.

With the years bound to `j` and `k`, `h` and `l` are free to scroll the project graph, which is what the all-time view needs: every project a writer has ever started is on that list. `gg` and `G` reach its ends, the title states which slice is on screen, and the bars stay scaled against the largest in the whole list so scrolling does not quietly rescale them. The dashboard is read-only, so it shows no mode indicator.

Bars are drawn with eighth-width block characters, so a short bar is not rounded away to nothing and a long one is not quantised to whole columns. A value that is not zero always draws at least one eighth.

The category bar sits above the year graph, since it describes the selected scope like everything above it, where the year graph spans all of them. Its key is drawn above the bar: the key is read first, and putting it underneath would place the explanation after the thing needing explained. The bar itself is two rows deep so the colours read as bands rather than a thin line.

The category bar hands out its cells largest-remainder first, so the segments add up to the full width instead of leaving a gap to rounding. Shares are taken against the sum of the positive categories, not the year's net total: a revision-heavy category can net out below zero, a negative slice of a bar means nothing, and such a category is named beneath the bar instead of being silently dropped. The legend wraps onto as many lines as it needs and the panel grows to fit, because a legend that quietly loses its last entries is worse than one that takes an extra line.

Creating a project that is already complete and predates 2024 is out of scope.

## The leader menu

Holding the leader key without following it shows a menu of what it can be followed by. Pressing leader is the moment the question arises, so that is when the answer belongs on screen, and a writer should not have to remember a second key to ask.

It lists only the leader sequences. At that moment the question is what this prefix does, and the full reference would bury the answer among two dozen keys that have nothing to do with the prefix; `<leader>?` opens the full reference from there. The Enter line names what submitting would do on the screen in hand, since it means something different on each.

The menu waits a short pause before appearing rather than arriving the instant leader is pressed. A menu that appears and vanishes inside a fast `<leader><CR>` is a flicker, not help, and submitting an entry is the most repeated action in the application. One hundred milliseconds reads as immediate while still being long enough that a sequence typed at speed never flashes it; the delay is a single constant in the event loop, and zero would make it appear on every press.

The menu does not swallow the key that dismisses it, which the full help deliberately does. The full help is opened to be read, so the keystroke that closes it is about closing it. The leader menu is shown while a sequence is half-finished, so the next key is the rest of that sequence and has to reach it.

## Keybindings

Leader is space. No control combinations are used anywhere.

| Key | Action |
| --- | --- |
| `?` | help popup, context-aware per screen |
| `<leader>` alone | after a short pause, a menu of what leader can be followed by |
| `<leader>?` | the full help from the leader menu |
| `<leader>e` / `<leader>h` / `<leader>p` / `<leader>c` / `<leader>s` | entry, history, projects, categories, stats |
| `j` `k`, `gg`, `G` | movement; in any form `j` and `k` walk between fields, like `Tab` |
| `h` / `l` | step a choice field: the entry form's project, a project's category and status, a category's type; switch pane on the categories screen |
| `Tab` / `Shift-Tab` | next and previous field in a form |
| `i` / `a`, `Esc` | begin and end editing a field; on a choice field, narrow its list |
| `/` | filter a list; it narrows rather than stepping between matches, so there is no `n` |
| `A` | widen a list from open projects to all of them, on the entry form and the projects screen |
| `o` | create a record on the projects or categories screen |
| `x` | cycle status on the projects list: open, completed, abandoned |
| `dd` | delete, then `y` to confirm; anything else keeps the row |
| `<leader><CR>` | submit a form |
| `j` / `k`, `[` / `]` | step the dashboard's scope: older year, newer year, then all time |
| `0` / `$` | on stats, this year / all time |
| `q` | quit, confirming when a form has unsaved input |

Submit is `<leader><CR>`. The requirements originally asked for `Ctrl-Enter`, which predates the no-control rule added afterwards; `mvp_design_doc.md` has been updated to match, so nothing in the app binds a control combination and the keymap is portable across mac, linux and windows.

## Theme

Melange, mapped by role so the palette is referenced in one module rather than spread through the widgets.

| Role | Colour |
| --- | --- |
| background | `a.bg` #292522 |
| popup and float background | `a.float` #34302C |
| selected row | `a.sel` #403A36 |
| borders, inactive text | `a.ui` #867462 |
| labels, secondary text | `a.com` #C1A78E |
| primary text | `a.fg` #ECE1D7 |
| focused border, accent | `b.cyan` #89B3B6 |
| completed, positive | `b.green` #85B695 |
| abandoned | `c.red` #BD8183 |
| warning, negative session | `b.yellow` #EBC06D |
| error | `b.red` #D47766 |
| graph series rotation | `b.blue`, `b.magenta`, `b.cyan`, `b.yellow`, then the `c` row |

Completed bars on the current-year projects graph are green as specified, which is why green is reserved and not part of the series rotation. Abandoned bars take the muted `c.red` rather than the brighter `b.red`, so abandoning a piece reads as a neutral outcome rather than an error.

## Build sequence

1. Cargo scaffold, schema, migrations, and the `rusqlite` access layer.
2. Importer, validated by comparing its report against the dry-run figures above. Run against a scratch database via `--db` first.
3. Entry form, including the session arithmetic and the negative case.
4. Entry history with edit and delete.
5. Projects and categories screens.
6. Stats dashboard with year stepping.
7. Help popup and theme pass.

Each step after the first is independently usable, so the import can be checked against the spreadsheets before any of the UI exists.
