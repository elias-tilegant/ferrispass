# P1a.05: Search ranking

Search in the open vault now lists the entry you are plainly asking for first, and every new search starts on its first match. Typing "nhq" in the main window puts NHQ 100 at the top and selects it; typing "nhp 100" puts the NHP 100 systems above everything that merely mentions them. The ranking lives in `src/app/search/rank.rs`, the selection in `src/app/state/select.rs`.

Started from `p1a-04-entry-kinds` at `f26d44d` on 2026-11-01, because the ranking reads the entry kinds from that step.

| Commit | What |
|---|---|
| `bba3837` | The four tiers, ties, and the main window's list using them |
| `047832d` | A new search selects its first match; explicit selection |
| `4809c01` | The copy an identifier is read from leaves out notes and protected fields from the start |
| `02da559` | This page, and "favorite" spelled the same way everywhere in the new code |
| `9bf129f` | The time recorded in the progress line |
| `docs(search): explain how a title starting with a SID and client ranks` | The open question on such titles below, and this table |

## The four tiers

The text search decides whether an entry matches and how well. The ranking then sorts the matches into tiers:

1. **Exact identifier.** The query names an SAP system by its SID and client, or any entry by its whole title. "nhp 100", "NHP 100" and "nhp   100" all name NHP 100. "nhp" alone does not: it names every NHP system, so it ranks like any other text.
2. **Favorites** whose text matches.
3. **Recently used** entries whose text matches, the most recent first. "Recently used" is the same session log the Recently used view shows: entries whose password or username was copied since the vault was unlocked.
4. **Everything else**, by how well the text matches, in exactly the order search gave before this step.

Inside a tier, favorites come first, then recently used entries (most recent first), then entries by title, then by id. The text score only orders the last tier. The first three hold entries the user has singled out, by naming them, starring them or using them, and they keep an order the user can learn instead of reshuffling with every letter typed.

A favorite or a recently used entry has to match the text to rank at all. An empty search lists nothing, as before.

### Where the SID and client come from

They come from the entry kinds (`kinds::classify`), so a title like "NHP 100 PRD" and the custom fields `SAP_SID` and `SAP_CLIENT` count the same, with the fields taking precedence as they do there. An entry titled "Nordhafen production" with those fields set to NHP and 100 is found by "nhp 100", although no word of its text matches.

Custom fields are still not search text. The SID and client count for the exact identifier and for nothing else.

A protected custom field counts for nothing at all. For an entry with protected fields, the ranking works out the kind from a copy of the entry without them and without its notes. The copy is built field by field, so neither is ever copied, and every field of the entry is named there: a new one does not compile until someone decides whether it may count. A protected `SAP_SID` or `SAP_CLIENT` is therefore treated as not set. Otherwise a search would reveal which entry holds a protected value: type guesses and watch which entry turns up as an exact match. A title "NHP 100" with a protected `SAP_CLIENT` of 200 is NHP 100 for the ranking, and "nhp 200" does not find it.

Deleted entries never show up, not even by their identifier.

## Ties

Two results can stand for the same thing, typically the same SID and client at two customers, or two logins with the same title. `tie_count` takes a ranked list and says how many results stand for the same thing as the first row, the first row included: the same SID and client (read without protected fields, as above), or the same title compared the way identifiers are. It is 1 when the first row is unique and 0 for no results.

Ties are ordered like everything else in a tier: favorite, then recently used, then title. Nothing shows the count yet. It is meant for a footer in the quick access panel, so the user knows the row Return would act on has a twin.

## The selection

Every new or changed search selects its first row, even when the entry selected before is still in the list. Before this step the selection stayed on that entry. Typing the same query again is not a new search and leaves the selection alone. Clearing the search, opening a group and switching between library views select the first row, as they did before.

### Explicit selection

The app now also knows whether the selected row was picked by hand. `select_entry`, which a click on a row calls, records the entry together with the list it was picked in. `AppState::selection_is_explicit` is true only when all three hold:

- A vault is open and the list shows a selected row. With no row shown there is nothing to have picked, even when no choice is recorded either.
- That row is the entry the user picked. What counts is the row the list shows (`shown_entry_id`), not the raw `selected_entry_id`, which can point at an entry that is no longer in the list while the first row is shown in its place. So a choice of an entry that is not in the list never makes a production system in the first row count as picked.
- The list is the very one the choice was made in. Every place that rebuilds the list (a search, a group, a library view, an edit, a delete, a sync merge) puts a new list there, and from then on the first row is the app's pick again, even when it is the same entry. A test reads the source of `src/app/state/` and fails if a place that sets the selection does not build a new list in the same function.

The choice is also cleared when the active vault changes: when it is parked to switch to another vault, and when a vault is opened, also the same vault again after a lock. Switching back to a parked vault brings back the same list and the same selected row, so without the reset on parking a choice made before the switch would still count.

The old main window looks no different for it. It has no arrow-key navigation in the list, so a click is the only way to pick a row by hand there. Nothing reads `selection_is_explicit` yet. It is there for the rule that Return never acts on a production system after an empty search unless the user picked it, which the action table will hold.

## What later steps add

- **Window context.** A tier between the exact identifier and the favorites, for entries that fit the window in front: its title (read through the accessibility API), the host of the page in a browser, and the kind of app (SAP GUI to SAP systems, a VPN client to remote access, a terminal to servers). It orders results and never types anything.
- **Personal before team.** Once search covers every unlocked vault, a tie prefers the personal vault's entry, after favorites and recently used and before the title.
- **The panel.** Results that arrive late in an asynchronous search must not move the selection; that belongs to the panel's search. The panel's arrow keys and Command-1 to 9 have to select through `select_entry`, like a click, or what they pick will not count as picked.

## Open

Whether the entry list and the detail panel should show a kind that only protected fields establish is not decided here. The ranking ignores protected fields; display still classifies the whole entry.

The entry kinds read a title that starts with a SID and a client as that system, whatever follows, so "NHP 100 Nordhafen alt" is NHP 100 just as "NHP 100 PRD" is. For "nhp 100" both are exact identifiers, and inside that tier a favorite comes first: a starred "NHP 100 Nordhafen alt" is listed above an unstarred "NHP 100 PRD" and is the row a new search selects, and `tie_count` counts it with the other NHP 100 entries. Whether a title with more text after the client should name the system for the ranking, or the identifier tier should order its entries by something other than favorites first, is not decided here. The sample vaults in the tests title the old entry "Nordhafen alt (NHP 100)", which does not start with the SID and so is not read as a system.

## Checks

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass (only the known future-incompat note for `block` 0.1.6) |
| `cargo test --locked` | pass, no new `#[ignore]` |
| `cargo audit` | pass, with the warnings the base branch already has |
| Em and en dash check over tracked files and the commit messages since the base | no hits |

The existing tests of the text search, `selection_tests.rs`, `park_tests.rs` and the characterization tests are unchanged and pass. The text search's own entry point, `ranked_entries`, is now built for tests only: the app lists results through the ranking, whose last tier `the_text_tier_keeps_the_text_order` holds to it.

The mutation probes, each run on a clean tree and reverted, are listed with their failing output in [PR #18](https://github.com/elias-tilegant/ferrispass/pull/18).
