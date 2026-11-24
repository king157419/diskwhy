---
name: diskwhy
description: Free up space on a Windows C: drive safely with the DiskWhy command line (diskwhy-cli.exe). Use when the user says the C drive / system disk is full, asks what is taking space, wants caches or junk cleaned, or mentions DiskWhy / C 盘满了 / 清理 C 盘 / 磁盘空间不够. Scans read-only, shows what is safe to delete, dry-runs, and only deletes after the user agrees.
---

# DiskWhy: clean the C: drive safely

`diskwhy-cli.exe` scans a drive, explains every big item with a curated rule library, and cleans through
guard rails (protected paths, tier gates, in-use / running-program / recently-modified checks, action log).
Full reference: `docs/CLI.md` in the DiskWhy repo. Run `diskwhy-cli --help` if unsure about a flag.

Locate the exe first: `diskwhy-cli.exe` on PATH, or `<repo>\target\release\diskwhy-cli.exe`.
No admin rights needed. Always pass `--json` and parse stdout (one UTF-8 JSON document); progress is on stderr.

## Workflow (do not skip steps)

1. **Scan** (read-only, about 1 minute for a full C: drive):
   `diskwhy-cli scan --json`
   Read `drive.free_bytes` / `drive.total_bytes`, `totals.safe.cleanable_bytes`, `totals.review.cleanable_bytes`, and `candidates`.

2. **Show the user** a short table in their language:
   - Free space now.
   - The **safe** list (`tier == "safe"` and `cleanable == true`): `label`, `cleanable_bytes` (human-readable), `comes_back`; and the safe total.
   - Optionally the biggest **review** items (`tier == "review"`) separately, with `lose_label` / `lose` — clearly marked as "needs your decision". Do not propose them for deletion on your own.

3. **Dry run** what the user wants (nothing is touched):
   - Safe tier: `diskwhy-cli clean --safe --json`
   - Specific items: `diskwhy-cli clean --ids id1,id2 --json`
   Report `totals.will_delete_bytes`, the per-item `dest` (delete / recycle / stash), `refused`, items with `blocked`,
   and skipped files grouped by `skipped_by_reason` (e.g. in use by which `processes`, modified in the last 30 min).
   Suggest closing the named programs if many files are in use; never close programs yourself.

4. **Ask for explicit agreement.** Only after the user clearly says yes, run the **same command with `--yes`**:
   `diskwhy-cli clean --safe --yes --json`

5. **Report** from `result`: `freed_bytes` (space actually freed on the drive), counts `ok` / `skipped` / `failed`,
   `by_dest`, top skip reasons, and `log_path` (and `manifest_path` if something went to the stash).
   Exit code `3` after `--yes` is normal (some files skipped); `2` means nothing was done.

## Hard rules

- Never run `clean --yes` without the user's agreement to that specific dry-run result.
- **Never pass review-tier ids unless the user named those items themselves.** Review items are never deleted directly
  (`--to delete` is refused for them); they go to the recycle bin (default) or the stash (`--to stash`).
- Never try to work around a refusal (keep-tier, protected, unknown, tool-only): explain the reason and, if given, the `how_detail` (e.g. a built-in tool or command the user can run).
- Do not delete files by hand (Remove-Item, del, rm) as a substitute for DiskWhy.
- If unsure what a folder is: `diskwhy-cli explain "<path>" --json` (shows `tier`, `what`, `lose`, `cli_refusal`, `candidate_id`).
- `--yes` runs can take minutes; do not kill the process midway.

## Exit codes

`0` ok, `1` usage error (fix the command), `2` nothing done / refused (read `error` / `refused` / `warnings`), `3` partial.
