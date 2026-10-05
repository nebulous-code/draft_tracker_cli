# Sandbox test walkthrough

Four behaviours exist only as passing tests and have never been exercised by hand: the stale-total refusal, `import --reset`, abandoned end dates, and backup pruning. This walks through all four against demonstration data, so none of it touches the real log.

## Before anything: use `--config`, never a bare `--db`

`--db` overrides the database and nothing else. `backup_directory` and `name` still come from the config file, so `draft-tracker --db /tmp/demo.db` writes a backup of the demo database into the real backup directory under the real `draft_tracker-` prefix, where it counts against `backups_to_keep`. Every command below goes through a separate config file instead.

## The sandbox

```
~/draft_tracker_sandbox/config.toml          database, backups, name = "sandbox", backups_to_keep = 3
~/draft_tracker_sandbox/demo.db              18 projects / 149 sessions / 171,725 words / 188.00 hours
~/draft_tracker_sandbox/import_source/       sessions.csv + projects.csv, a 4-row import
~/draft_tracker_sandbox/backups/             empty
```

Give yourself a handle:

```sh
alias dts='draft-tracker --config ~/draft_tracker_sandbox/config.toml'
```

Check it before starting. Every row should say `config file`, and `name` must be `sandbox`. If any row points at `/the_agency`, stop.

```sh
dts config
```

To return to a clean slate at any point:

```sh
rm -f ~/draft_tracker_sandbox/demo.db* ~/draft_tracker_sandbox/backups/*
cd ~/Documents/Code/draft_tracker
cargo run --example demo_data -- ~/draft_tracker_sandbox/demo.db
```

## Test 1: the stale-total refusal

The case where a wrong answer silently writes a wrong number, so it matters most. Needs two terminals.

Terminal A:

```sh
dts
```

The TUI opens on the entry form. Press `l` until the project reads **Salt Lantern**. It should say `logged so far 15,310`. Leave it sitting there.

Terminal B:

```sh
dts log "Salt Lantern" 16000 1.0 -y
```

That writes a 690-word session. Terminal A still shows the stale 15,310.

Terminal A, now entering a session as if nothing had happened:

- `j` to Hours, `i`, type `1.0`, `Esc`
- `j` to Project WC, `i`, type `17000`, `Esc`
- submit with `<leader><CR>` (space, then Enter)

Expected: the submit is refused once. A warning reads *"another session added work; the figures have been recalculated, check them and submit again."* `logged so far` jumps to 16,000 and Session WC drops from 1,690 to **1,000**. Press `<leader><CR>` again and it writes 1,000.

The failure to watch for is it writing **1,690**, the difference against the stale baseline. That is the corruption this guard exists to stop.

### The screen-change variant

With a TUI open in A, log something from B, then press `<leader>p` in A. Expected: an info flash, *"another session changed the log; reloaded,"* and the projects table already showing the new work. This check fires on screen change and submit only, not per keystroke.

## Test 2: `import --reset`

```sh
dts import ~/draft_tracker_sandbox/import_source --dry-run
```

Reports 4 entries, 2,390 words, 5.00 hours, 2 projects, and writes nothing. The source includes a -210 word row, so a trimming day can be seen surviving the import.

```sh
dts import ~/draft_tracker_sandbox/import_source
```

Expected: refusal, *"the database already holds data; pass --reset to replace it, or --db to import somewhere else."* Exit code 1.

```sh
dts import ~/draft_tracker_sandbox/import_source --reset
```

Expected: `Backed up to .../sandbox-<timestamp>.db first.` printed before anything else, then the import. The 171,725 words are now gone, replaced by 2,390.

### Then verify the recovery

This is the half worth doing, because it is the procedure to follow at 2am after a real mistake.

```sh
dts status                                 # 2,390 words
ls -1 ~/draft_tracker_sandbox/backups      # the pre-reset backup is the newest
cp ~/draft_tracker_sandbox/backups/<newest>.db ~/draft_tracker_sandbox/demo.db
dts status                                 # 171,725 back
```

## Test 3: abandoned end dates

Open `dts`, go to `<leader>p`, press `A` to widen from open projects to all 18.

**Nine Doors** is the abandoned one. Select it and check the detail pane: `Start / End` should show a real date on both sides. Compare against **Low Tide**, which is open, where End should be a dash.

Then exercise the transition. Select Low Tide and press `x` to cycle its status. Open has no end date; the moment it becomes completed or abandoned, End should populate with its last entry's date, and cycling back to open should clear it again. Both closed states behave identically, which was the requirement: abandoned carries the same calculation dependencies as completed.

## Test 4: backup pruning

The sandbox sets `backups_to_keep = 3`, so this is observable in five commands rather than twenty.

```sh
ls -1 ~/draft_tracker_sandbox/backups     # empty
for i in 1 2 3 4 5; do
  dts log "Salt Lantern" $((15310 + i*400)) 1.0 -d 2026-03-0$i -y
  echo "$i: $(ls ~/draft_tracker_sandbox/backups | wc -l) backups"
done
```

Expected: the count climbs 1, 2, 3 and then stays at 3, keeping the three newest. This is the one that had the ordering bug where the newest file sorted as oldest and pruning deleted the wrong thing, so check the timestamps in the surviving filenames really are the three most recent.

Then the skip-when-unchanged logic:

```sh
dts backup      # writes, printing integrity_check ok with counts
dts backup      # "Nothing has changed since .../sandbox-<timestamp>.db."
```

Backups are written after each save, so with the real config's `backups_to_keep = 20` that is roughly a day or two of active writing. The ZFS snapshots provide time depth; `backups_to_keep` only protects against the last few mistakes.

## Cleaning up

```sh
rm -rf ~/draft_tracker_sandbox
```

## What was validated ahead of this, and what was not

Validated by running it: the sandbox isolation, pruning capping at 3 and keeping the newest, the import refusal and `--reset` backing up before replacing, restore-from-backup returning the full dataset, the unchanged-backup skip, and which commands write backups at all. `status` and `config` do not; `backup`, `log`, and opening the TUI do.

Not validated, and the reason this document exists: Test 1, which needs two live terminals, and Test 3, which only means something when the detail pane can be seen changing.
