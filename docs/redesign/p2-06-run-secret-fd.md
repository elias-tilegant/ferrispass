# P2.06: run a program with a secret on a file descriptor

Scripts and agents used to get a secret out of FerrisPass with `entry secret --reveal`, which prints it, so that they could hand it to a program as an argument, in a variable or through a file. `ferrispass-cli run` hands it to the program directly instead:

```sh
ferrispass-cli --vault team.kdbx --touch-id run --id UUID --secret-fd 3:password -- node server.js
```

starts `node server.js` with the entry's password readable on descriptor 3 up to end of file. The secret never passes through the script or the agent, and it is in no argument, no environment variable and no file. For a program that can only take a secret from the environment, `--env NAME=FIELD` is the explicit fallback, with a warning. While the program runs, `run` passes SIGINT and SIGTERM on to it and returns its exit code. The Touch ID prompt says which fields go to which program, and `entry secret --reveal` prints only to a terminal now. `docs/cli.md` has the section "Handing a secret to a program" with an MCP server as the example.

Started from `rebuild` at `da54801` on 2026-11-11, in three parts on one branch: the command and its descriptor module, the Touch ID prompt and `--reveal`, then `--env`, signals and the documentation.

| Commit | What |
|---|---|
| `6311f19` | `src/platform/secret_fd.rs`, its place in `src/platform/mod.rs` and on the list of audited platform modules, and `libc` for every Unix |
| `18e0731` | `run` in `src/cli.rs` and the process tests in `tests/cli_run.rs` |
| `da81cd8` | The Touch ID prompt for `run`, and `--reveal` only on a terminal |
| `8a718a4` | SECURITY.md: the CLI descriptor in scope, its row in the security contract and under "Unsafe code" |
| `ebd0e72` | The leak scan also reads names a macro captures in a format string |
| `ab035f7` | The program no longer gets the descriptor the master password was read from |
| `768ab7c` | The test of the `unsafe` blocks holds the module to the `libc` functions its header names |
| `7ea6675` | SECURITY.md says that only ASCII letters keep a name in the prompt |
| `d6b44b4` | `--env`, the rule that a field goes to the program once, and the master password's descriptor has to be open before anything is unlocked |
| `b84b457` | The master password's descriptor stays closed in the program on 11 as on 9 |
| `6ef77ec` | `run` passes SIGINT and SIGTERM on to the program |
| `dc223fe` | SECURITY.md: `--env` and the signal handler |
| `2e288f3` | The test of the field order in the prompt: a field named twice never reaches it |
| `99c0a83` | `docs/cli.md`, README and `docs/architecture.md` |
| `c134e95` | The signal test waits for its program before it asserts, so a failing run leaves nothing behind |
| `9207552` | This page |
| `2b7340a` | The master password's descriptor stays closed in the program with `--env` alone |
| `ee35488` | SIGTERM reaches the program with `--env` alone |
| `f16ea34` | The test that the `--env` warning is written before the program starts |
| `5f435e2` | This page lists those tests |
| `545e531` | Whether a number to close is open is asked with `fcntl(F_GETFD)`, the fifth `unsafe` block |
| `11367dc` | `custom:` with an empty key is refused before Touch ID; a `--env` warning that cannot be written ends `run` with 7 before the program starts |
| `9b66e9d` | SIGINT reaches the program with `--env` alone |
| `b59ffce` | The limits near the open file limit and between the reap and the clearing of the pid, and this page |
| `279ed68` | The comment of the test for the `--env` warning wrapped at the file's width |
| the last one | This page lists those two |

## What `run` does

`run --id UUID` names one entry, by its UUID. Each `--secret-fd FD:FIELD` puts one field on one descriptor, from 3 to 255 (close to the open file limit, see Limits); each `--env NAME=FIELD` puts one into one variable. A field is `password`, `totp` or `custom:KEY` with a `KEY` that is not empty, and a custom field has to be protected, as for `entry secret`. Everything after `--` is the program and its arguments, started directly, not through a shell.

Before anything is unlocked, `run` checks the command line: the UUID, the syntax of each option, the field names, the descriptors, that no descriptor is named twice, that each field goes to the program once (on a descriptor or in a variable, not both, and not in two variables), that each variable is named once and has a name a shell takes, and that the descriptor `--master-password-fd` named is open. Each of these fails with exit code 6. Then `run` unlocks the vault, reads the released fields into buffers that are wiped when dropped, closes the vault and starts the program. It prints nothing of its own on stdout, so the program has stdin, stdout and stderr to itself, and an MCP server can talk over them. Once the program has started, `run` exits with its exit code, or with 128 plus the signal that ended it. A secret the program did not read in full is reported on stderr as a byte count.

## The descriptor module

`src/platform/secret_fd.rs` is the third audited module under `src/platform/`. Its header lists the steps; in short:

- One pipe per descriptor. Each read end is copied with `fcntl(F_DUPFD_CLOEXEC)` above the highest target and the original is closed, so no read end sits on a target when the program starts. Every number up to the highest target that is still free is held with `/dev/null` during the spawn, so the descriptors `Command::spawn` opens for itself land above the targets too.
- Between fork and exec the child runs `dup2(copy, target)` for each target and `close` on the copy, then `close` on each number it must not pass on that is not a target: for `run`, the descriptor the master password was read from. Nothing else happens there: no allocation, no lock, no formatting. A failed `dup2` fails the spawn.
- A number to close has to be open in this process, and stays open until the spawn has returned, so nothing the spawn opens can have that number. `spawn_with_secret_fds` refuses one that is not open, and the CLI asks before it unlocks the vault. Whether a number is open is asked with `fcntl(F_GETFD)`, which only reads the descriptor's flags and fails with `EBADF` for a number that is not open. Not `stat` on `/dev/fd/N`: on macOS that fails now and then with `EBADF` for an open descriptor while other processes look up the same number; with the CLI started 6400 times from 16 shells in parallel, 1 to 3 runs in each of three rounds refused an open master password descriptor that way, and none with `fcntl`.
- Right after the spawn, the parent closes its copies and the held descriptors, whether the spawn worked or not. Then one thread per pipe writes the secret and closes the write end, so a secret larger than the pipe buffer arrives while the program reads, and a program that exits without reading leaves no writer waiting.
- Errors carry descriptor numbers, byte counts and the kind and OS code of what the system refused, never the bytes, an `io::Error`'s message or a path.

The module has five `unsafe` blocks: `fcntl(F_GETFD)` in the open check, `fcntl(F_DUPFD_CLOEXEC)`, the `pre_exec` closure with `dup2` and `close`, `sigaction` and `kill`. Tests read the file and hold it to that, below.

## The Touch ID prompt

With Touch ID, the prompt says what `run` gives to which program, for example "Give node the password and a custom field of entry 1b4e28ba-2fa1-11d2-883f-0016d3cca427 from team.kdbx". The program and the vault appear by their file names, only when a name is 1 to 40 characters from `A-Z a-z 0-9 . _ -`, otherwise as "a program" and "a vault". The entry is the canonical form of its UUID; Touch ID runs before the vault is open, so the title is not known yet. Each field appears by its kind, those on descriptors first, then those in variables, each in the order given, and custom fields are counted, never named. A variable's name never appears, and nothing else from the command line does, so a script cannot write its own words into the prompt. Every other command keeps "Unlock FerrisPass vault".

## `--env` and its warning

`--env NAME=FIELD` sets the variable `NAME` for the program only: it is not exported to the shell that started `run`. It is still an environment variable, readable by programs running as the same user and passed on to everything the program starts, so `run` prints one line per variable on stderr before the program starts, also with `--format json`:

```text
warning: GITHUB_TOKEN is set only for this program, not exported, and readable by programs running as your user
```

The warning names the variable, never the value. If it cannot be written, the program does not start and `run` exits with 7, the code for an I/O failure. `NAME` starts with a letter or `_` and has only letters, digits and `_`.

With `--env` alone nothing goes on a descriptor, and the program still starts through the descriptor module: it does not get the master password's descriptor either, and SIGINT and SIGTERM reach it as they do with `--secret-fd`.

## Signals

While the program runs, `run` passes SIGINT and SIGTERM on to it and keeps waiting, so the program decides how it ends and `run` still returns its exit code. Before, a SIGTERM killed the CLI and a SIGINT ended it with 130, and the program ran on without anyone waiting for its exit code.

The handler is installed with `sigaction` before the spawn. It stores the signal in one atomic, reads the program's pid from a second one and calls `kill` when the pid is above 0; that is all it does. Right after the spawn, the pid goes into the second atomic and a signal that arrived in between is sent. Both sides store first and read second, so such a signal reaches the program once or twice and is never lost. The guard that holds the old actions is an argument of the spawn, so it exists before the program does, and it is dropped right after the program has been reaped, before the writers are joined: the pid is cleared first, then the old actions are back. A signal in the few instructions between the reap and the clearing of the pid still goes to the reaped pid (Limits). That happens on every path, also when the spawn or the wait fails. For SIGINT the old action is the Ctrl+C cleanup of `launch`, which `run` has nothing to clean up for.

## `--reveal`

`entry secret --reveal` prints a secret only when stdout is a terminal. With stdout on a pipe or in a file it fails with `reveal_needs_terminal`, exit code 6, prints nothing on stdout, and the message points to `run --secret-fd`.

## What the tests show

The unit tests in `src/platform/secret_fd.rs` start `/bin/sh` with secrets on descriptors and read what it got: each secret on its descriptor, descriptors given in any order and crossed onto each other's numbers, a pipe whose read end already has the target number, 200 KiB to a program that reads late and to one that never reads, a missing program on every target from 3 to 24, no end of a pipe left open after a failed spawn, nothing in the program's arguments or environment, a number to close that is not open refused before anything is opened. The signal tests call the handler's function before the pid is known and check that a shell in the same process group does not get the signal, then that the program gets it after the spawn, and that the old actions and both atomics are back after the wait and after a failed spawn. Four tests read the file itself: `pre_exec_only_duplicates_and_closes`, `the_signal_handler_only_forwards`, `secret_fd_has_exactly_the_declared_unsafe_sites` and `secret_fd_never_logs_the_secret`.

The unit tests in `src/cli.rs` run `run` up to the Touch ID prompt with a store that records the reason: the reason word for word, caller text kept out, the UUID in canonical form, the vault's name, every field in order with descriptors first, custom fields counted, and every command line that has to fail before Touch ID failing there.

`tests/cli_run.rs` runs the CLI as a process against a vault on disk, with an empty home folder of its own: the password on 3, TOTP and custom fields, the exit code passed through, the program's environment and arguments, unknown fields and `custom:` with no key refused before the unlock, the master password's descriptor closed in the program on 9 and on 11, and on 9 also with `--env` alone, where nothing goes on a descriptor, and refused when it is not open, a sentinel password on every error path in JSON and human output with no four characters of it on stdout or stderr, `--env` setting only its variable and leaking nowhere else, the warning once per variable and before the program starts (with stderr on a socket shut down for writing, writing the warning fails, the program never starts and the CLI exits with 7), SIGTERM and SIGINT passed on with the program's exit code returned, both also with `--env` alone, the CLI waiting for a program that takes a second to end, 143 for a program the signal kills, `--reveal` refused on a pipe, and `launch sap` removing its staged file and exiting with 130 when SIGINT hits it. The last one runs the CLI in a sandbox that refuses every Mach lookup, so the `open` it starts cannot reach Launch Services and no SAP GUI opens on the Mac running the tests.

The characterization tests and `src/i18n/tests.rs` are unchanged and pass.

## Mutation probes

Each rule was checked by taking it out on a clean tree and running the tests named; the changed lines and the red output are in [PR #30](https://github.com/elias-tilegant/ferrispass/pull/30).

| # | Rule taken out | Red |
|---|---|---|
| 1 | the secret also in the program's environment | `secret_is_not_in_argv_or_environment`, `run_never_puts_the_secret_in_argv_or_env` |
| 2 | targets below 3 refused | `descriptors_below_three_are_refused` |
| 3 | the write end closed after the secret | `write_end_is_closed_after_the_secret` |
| 3a | read ends copied above the targets | `target_fd_3_works_when_the_pipe_got_fd_3`, `two_pipes_land_on_3_and_4_in_either_order` |
| 3b | writing after the spawn, on threads | `a_secret_larger_than_the_pipe_buffer_arrives_complete` |
| 3d | the parent's read ends closed right after the spawn, and on a failed spawn | `run_returns_when_the_child_exits_without_reading`, `spawn_failure_closes_every_descriptor` |
| 3e | the upper bound and `try_from` (`checked_add` stays green: the bound before it caps the sum) | `run_rejects_target_fds_outside_3_to_255_and_duplicates`, `targets_above_255_are_refused` |
| 3f | custom fields counted by expression | `run_reason_counts_distinct_custom_fields` |
| 3c | the UUID check, the key, the vault name and every field in the prompt | `run_rejects_an_id_that_is_not_a_uuid`, `run_reason_never_takes_caller_text`, `run_reason_sanitizes_the_vault_name`, `run_reason_names_every_released_field` |
| 4 | `--reveal` only on a terminal | `reveal_is_refused_when_stdout_is_not_a_terminal`, `reveal_error_points_to_run` |
| 5 | the program in the prompt | `run_reason_names_program_vault_and_entry_id` |
| 6 | the `--env` warning | `env_fallback_prints_the_warning_once` |
| 7 | no buffer in an error or a format string | `errors_display_and_debug_hold_no_secret`, `run_never_leaks_the_secret_into_output_or_errors`, `secret_fd_never_logs_the_secret` |
| 8 | only `dup2` and `close` after fork | `pre_exec_only_duplicates_and_closes` |
| 9 | the declared number of `unsafe` blocks | `secret_fd_has_exactly_the_declared_unsafe_sites` |
| 10, 11 | the master password's descriptor closed in the child, but not a target on that number | `run_keeps_the_master_password_descriptor_from_the_program`, `a_number_to_close_that_is_a_target_keeps_its_secret` |
| 12 | the `libc` functions the header names | `secret_fd_has_exactly_the_declared_unsafe_sites` |
| 13 | numbers to close below 3 refused | `numbers_to_close_below_three_or_past_any_descriptor_are_refused` |
| H | numbers below the targets held during the spawn | `a_failed_exec_is_reported_whatever_the_target` |
| 14 | the handler calls `kill`, not `exit` | `the_signal_handler_only_forwards`, `run_waits_for_the_program_after_a_signal` |
| 15 | the guard puts the old actions back | `signals_are_restored_after_the_program_ends` |
| 16 | a field goes to the program once | `env_and_fd_cannot_name_the_same_field`, `the_reason_names_fields_from_env_and_fd_in_order` |
| 17 | a number to close has to be open, in the module and in the CLI | `a_closed_number_to_close_is_refused_before_touch_id`, `run_refuses_a_master_password_descriptor_that_is_not_open` |
| 18 | `child_started` sends what came before the pid; the handler stores before it loads | `a_signal_before_the_pid_is_known_reaches_the_child_after_the_spawn`, `the_signal_handler_only_forwards` |
| 19 | the `--env` value only in the environment | `the_env_fallback_leaks_the_secret_nowhere_else` |
| 20 | a variable named once, with a name a shell takes | `the_same_variable_twice_is_refused`, `an_invalid_variable_name_is_refused` |
| 21 | SIGINT passed on as well as SIGTERM | `run_passes_sigint_on` |
| 22 | 128 plus the signal for a program a signal ended | `a_program_killed_by_a_signal_gives_128_plus_the_signal` |
| 23 | the handler's comparison of the pid with 0 | `the_signal_handler_only_forwards`, `a_signal_before_the_pid_is_known_reaches_the_child_after_the_spawn` |
| 24 | the descriptor module also for a program that gets nothing on a descriptor | `run_with_only_env_keeps_the_master_password_descriptor_from_the_program`, `run_with_only_env_passes_sigterm_on` |
| 25 | the `--env` warning written before the spawn | `the_env_warning_comes_before_the_program_starts` |
| 26 | the guard clears the pid when it is dropped | `signals_are_restored_after_the_program_ends` |
| 27 | the master password's descriptor closed in the child also when nothing goes on a descriptor | `run_with_only_env_keeps_the_master_password_descriptor_from_the_program` |
| 28 | the guard handed to the spawn also when nothing goes on a descriptor | `run_with_only_env_passes_sigterm_on`, `run_with_only_env_passes_sigint_on` |
| 29 | the open check asks `fcntl` about each number | `a_closed_number_to_close_is_refused_before_touch_id`, `run_refuses_a_master_password_descriptor_that_is_not_open` |
| 30 | `custom:` with an empty key refused when the command line is checked | `run_refuses_unknown_fields_with_exit_6` |

Creating the guard after the spawn, the other half of probe 18, cannot be written: the guard is an argument of the spawn, so the change does not compile.

## Limits

- A master password kept in a file stays readable by its path to every program of the same user, the one `run` starts included; `lsof` on the CLI shows that path while the CLI holds the descriptor. A pipe or Touch ID leaves nothing to read again.
- The program inherits every other descriptor the CLI itself inherited without close-on-exec, as any program a shell starts does; which ones that are is the caller's choice.
- Ctrl+C in a terminal goes to the whole foreground process group, so the program gets SIGINT twice: once from the terminal, once from `run`.
- A signal that arrives in the moment between the spawn and the pid being known can reach the program twice as well, once from the handler and once from the delivery after the spawn. Of two different signals in that moment, the program gets the later one.
- A signal that arrives after the program has been reaped and the old actions are back meets those actions: a SIGTERM ends the CLI, a SIGINT ends it with 130, and the program's exit code is lost although the program has already ended. One that arrives between the clearing of the pid and the old actions coming back is dropped.
- Between the reap and the clearing of the pid there are a few instructions on the waiting thread. A signal in them goes to the reaped pid, which macOS hands out again only after a full cycle of pids. A `kill` that fails there changes `errno` under the code the signal interrupted, which at worst shows a wrong OS code in a warning line on stderr that holds only numbers. Closing that window would take `waitid` with `WNOWAIT` or saving `errno`, each one more `libc` function.
- While `run` waits, it takes SIGINT and SIGTERM even if its caller set them to be ignored, and the program starts with the default action for both. For SIGINT that was so before: the Ctrl+C cleanup of `launch` takes it when the CLI starts.
- A value given with `--env` is copied into the environment `Command` builds and into the program's environment, and neither copy can be wiped; only the CLI's own buffer is.
- If the program hands its descriptor to a process that outlives it and never reads, the writer of a secret larger than the pipe buffer waits for that process, and so does `run` after the program has ended. SIGINT and SIGTERM end the CLI then, since the program has been reaped.
- A short write shows only when the secret is larger than what the pipe holds. A smaller secret sits in the pipe whether the program reads it or not.
- On macOS `std::io::pipe` sets close-on-exec right after `pipe`, so a program another thread starts in that moment keeps both ends. The CLI starts no programs on other threads.
- The Touch ID prompt is tested through the reason handed to the store. What macOS shows around it needs a run by hand on a Mac with Touch ID.
- The structural tests read the source as tokens and do not see what a macro would generate; the closure and the handler have no macro, and the same tests fail on one. The leak scan finds the buffers by their names, `secret` and `secrets`; the same bytes under another name are left to the tests that look at what comes out.
- `entry secret` is refused on a non-terminal after the vault is unlocked, as the check lives where the field is read. A script that asks for it with `--touch-id` sees the Touch ID prompt first.
- The launch test needs `sandbox-exec`, which macOS still ships but calls deprecated.
- While it starts the program, `run` needs free descriptor numbers above the highest target: one per secret for the copies of the read ends, and two more for `Command::spawn`. Close to the open file limit it fails with exit code 7 after the unlock (with Touch ID, after the prompt) and before the program starts. Under `ulimit -n 256`, the soft limit a macOS shell has by default, one secret on 255 fails with `EINVAL` from the copy, on 254 with `EMFILE` while the numbers below are held, and on 253 with `EMFILE` from the spawn; 252 and below start the program as long as the caller holds nothing open above them. Nothing is handed over then, and no part of the secret is on stdout or stderr.
- After the program has ended, `run` reports a secret the program did not read in full as a `warning:` line on stderr. If stderr cannot be written at that point, the CLI ends with the panic code 101 instead of the program's exit code.
- CI builds and tests on macOS only. The module is written for any Unix, but nothing here builds it for Linux.

The agents spent about 5 hours on this, including review and fixes.
