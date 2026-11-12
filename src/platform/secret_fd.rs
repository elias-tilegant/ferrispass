//! Hands secrets to a child process on file descriptors, for
//! `ferrispass-cli run --secret-fd`.
//!
//! A secret in a program's arguments can be read by every process on the Mac
//! that lists processes, one in its environment by the program and by
//! everything it starts, and one in a file by whatever reads the disk later.
//! Here the program gets each secret on a pipe instead: this process writes
//! the secret into one end, and the program reads the other end on the
//! descriptor number it asked for, up to end of file. The secret is in no
//! argument, no environment variable and no file, and nothing here logs it or
//! puts it into an error.
//!
//! The steps, in this order:
//!
//! 1. The targets are checked. Each is a `u32` from the command line,
//!    converted to `c_int` with `try_from`, from 3 to 255, and no two are the
//!    same; 0, 1 and 2 are the program's standard streams. The numbers the
//!    program must not get (step 5) are converted the same way and have to
//!    be 3 or above, and each of them that is not also a target has to be
//!    open in this process: [`check_open`] looks for it in `/dev/fd`, which
//!    has an entry for each open descriptor, and a closed one is
//!    [`SecretFdError::NotOpen`]. Nothing is opened before that check.
//! 2. One pipe per target, from `std::io::pipe`. Both ends are close-on-exec.
//!    On macOS the standard library sets that flag right after `pipe`, so a
//!    program another thread starts in between would keep both ends. Nothing
//!    in the CLI starts programs on another thread.
//! 3. Each read end is copied with `fcntl(F_DUPFD_CLOEXEC)` to a number above
//!    the highest target (the highest target plus one, with `checked_add`, is
//!    the lowest the copy may get), and the original is closed. A read end on
//!    a target number would break step 5: the pipe for target 3 often gets
//!    descriptor 3 itself, and `dup2` of a descriptor onto its own number
//!    changes nothing, so it would keep close-on-exec and the program would
//!    lose it at exec. With two pipes, one read end could also sit on the
//!    other's target and be replaced before it is moved.
//! 4. Every number up to the highest target that is still free is held with
//!    `/dev/null`, close-on-exec, until the spawn has returned.
//!    `Command::spawn` opens descriptors of its own to learn about a failed
//!    exec. Held this way, they land above the targets too, so the child's
//!    `dup2` cannot replace them, which would make a program that does not
//!    exist look started.
//! 5. The spawn runs a `pre_exec` closure in the child, between fork and exec:
//!    for each target, `dup2(copy, target)`, then `close(copy)` if the copy is
//!    not the target. After that it closes each number the program must not
//!    get, unless the number is also a target, where `dup2` has already put
//!    the program's pipe. `dup2` leaves close-on-exec off the target, so the
//!    program keeps it across exec; the copies, the write ends and the held
//!    `/dev/null` descriptors all close at exec. A failed `dup2` returns its
//!    errno through `io::Error::last_os_error`, the spawn fails with it and the
//!    program never runs. After fork only async-signal-safe calls are allowed,
//!    and the closure makes no other call: no allocation, no lock, no
//!    formatting. `last_os_error` reads errno into an `io::Error` without
//!    allocating, as the standard library's own code after fork does.
//! 6. Right after the spawn, whether it worked or not, every copy from step 3
//!    and everything held in step 4 is closed. Without that, a writer would
//!    never get `EPIPE` when the program exits without reading, and waiting
//!    for it would hang.
//! 7. One thread per pipe writes the secret into the write end and then
//!    closes it, so the program reads to end of file. Writing starts after the
//!    spawn, each pipe on its own thread, so a secret larger than the pipe
//!    buffer (64 KiB on macOS) does not block before the program reads.
//! 8. [`RunningChild::wait`] waits for the program, then for the writers.
//!
//! The numbers the program must not get are descriptors this process
//! inherited without close-on-exec, which the program would otherwise inherit
//! in turn: for `run`, the one `--master-password-fd` named, which a program
//! could seek back to the start and read again. The caller keeps each of them
//! open until the spawn has returned. While a number is open here, nothing
//! the steps above open can get it: no copy from step 3, no `/dev/null` from
//! step 4 and no descriptor of `Command::spawn`'s own. So closing that number
//! in the child closes the inherited descriptor and nothing else.
//!
//! This process holds each secret only in a `Zeroizing` buffer, which the
//! writer wipes when it is done. A write that stops early, because the
//! program closed the descriptor or exited without reading it all, comes back
//! as [`SecretFdError::Write`] with the descriptor and the byte counts. No
//! error here carries a buffer, the message of an `io::Error` or a path: only
//! numbers, and the kind and OS error code of what the system refused.
//!
//! There are exactly two `unsafe` blocks in this file, and the `libc` calls in
//! them are `fcntl`, `dup2` and `close`:
//!
//! - in `copy_above`, `fcntl(F_DUPFD_CLOEXEC)` on a read end this module
//!   holds, and `OwnedFd::from_raw_fd` on the new descriptor, which nothing
//!   else owns;
//! - in `spawn`, `CommandExt::pre_exec` with the closure from step 5.
//!
//! `pre_exec_only_duplicates_and_closes` and
//! `secret_fd_has_exactly_the_declared_unsafe_sites` read this file and hold
//! it to that.

#![cfg(unix)]
#![allow(unsafe_code)]

use std::fmt;
use std::fs::File;
use std::io::{self, PipeReader, PipeWriter, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus};
use std::thread::{self, JoinHandle};

use libc::c_int;
use zeroize::Zeroizing;

/// The lowest target: 0, 1 and 2 are the program's standard streams.
const LOWEST_TARGET: c_int = 3;
/// The highest target.
const HIGHEST_TARGET: c_int = 255;

/// Why a secret did not reach the program. Every variant carries numbers
/// and, where the system refused something, the kind and OS code of that
/// refusal; never a secret, a message or a path.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SecretFdError {
    #[error("descriptor {0} is below 3; 0, 1 and 2 are the program's standard streams")]
    BelowThree(u32),
    #[error("descriptor {0} is above 255")]
    AboveLimit(u32),
    #[error("descriptor {0} is named twice")]
    Duplicate(u32),
    #[error("descriptor {0} is not open")]
    NotOpen(u32),
    #[error("could not set up the descriptors for the program: {0}")]
    Setup(Cause),
    #[error("could not start the program: {0}")]
    Spawn(Cause),
    #[error("descriptor {target} got {written} of {len} bytes: {cause}")]
    Write {
        target: u32,
        written: usize,
        len: usize,
        cause: Cause,
    },
    #[error("could not wait for the program: {0}")]
    Wait(Cause),
}

/// What the system refused with: the kind and, when there is one, the OS
/// error code. Kept instead of the `io::Error`, whose message can be any
/// text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cause {
    pub kind: io::ErrorKind,
    pub code: Option<i32>,
}

impl Cause {
    fn of(error: &io::Error) -> Self {
        Self {
            kind: error.kind(),
            code: error.raw_os_error(),
        }
    }
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(code) => write!(f, "{}", io::Error::from_raw_os_error(code)),
            None => write!(f, "{}", self.kind),
        }
    }
}

/// How a program started with [`spawn_with_secret_fds`] ended.
#[derive(Debug)]
pub struct Finished {
    pub status: ExitStatus,
    /// A [`SecretFdError::Write`] for each descriptor whose secret did not
    /// arrive in full: the program closed it early or exited without reading
    /// all of it.
    pub undelivered: Vec<SecretFdError>,
}

/// The program, and one writer per descriptor.
pub struct RunningChild {
    child: Child,
    deliveries: Vec<Delivery>,
}

enum Delivery {
    Writing {
        target: u32,
        len: usize,
        thread: JoinHandle<Result<(), SecretFdError>>,
    },
    Failed(SecretFdError),
}

impl RunningChild {
    /// The program's process id.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Waits for the program to exit, then for every writer. A writer is done
    /// by then or about to be: the program's ends of the pipes closed when it
    /// exited, and this process holds none.
    pub fn wait(mut self) -> Result<Finished, SecretFdError> {
        let status = self
            .child
            .wait()
            .map_err(|error| SecretFdError::Wait(Cause::of(&error)))?;
        let mut undelivered = Vec::new();
        for delivery in self.deliveries {
            let outcome = match delivery {
                Delivery::Writing {
                    target,
                    len,
                    thread,
                } => thread.join().unwrap_or(Err(SecretFdError::Write {
                    target,
                    written: 0,
                    len,
                    cause: Cause {
                        kind: io::ErrorKind::Other,
                        code: None,
                    },
                })),
                Delivery::Failed(error) => Err(error),
            };
            if let Err(error) = outcome {
                undelivered.push(error);
            }
        }
        Ok(Finished {
            status,
            undelivered,
        })
    }
}

/// Checks the targets as step 1 says, without starting anything. The CLI
/// calls this before it unlocks the vault.
pub fn check_targets(targets: impl IntoIterator<Item = u32>) -> Result<(), SecretFdError> {
    bounds(targets).map(|_| ())
}

/// Checks that each number is a descriptor open in this process, as step 1
/// says, without opening anything: `/dev/fd` has an entry for each open
/// descriptor (on Linux it is `/proc/self/fd`), and asking for one's
/// metadata opens nothing. The CLI calls this before it unlocks the vault.
pub fn check_open(numbers: impl IntoIterator<Item = u32>) -> Result<(), SecretFdError> {
    for number in numbers {
        if std::fs::metadata(format!("/dev/fd/{number}")).is_err() {
            return Err(SecretFdError::NotOpen(number));
        }
    }
    Ok(())
}

/// Starts `cmd` with each secret on a pipe whose read end the program has as
/// the given descriptor number, and without the descriptors numbered in
/// `close_in_child`. The steps are in the module header.
///
/// `close_in_child` is for descriptors this process inherited without
/// close-on-exec and must not pass on, such as the one the master password
/// was read from. Each of them that is not also a target has to be open in
/// this process when this is called and stay open until it returns, so that
/// nothing this function opens can take its number; one that is not open is
/// refused before anything is opened. A number that is also a target stays
/// open in the program, which gets its secret there.
pub fn spawn_with_secret_fds(
    cmd: Command,
    secrets: Vec<(u32, Zeroizing<Vec<u8>>)>,
    close_in_child: &[u32],
) -> Result<RunningChild, SecretFdError> {
    check_targets(secrets.iter().map(|(target, _)| *target))?;
    closable(close_in_child)?;
    check_open(
        close_in_child
            .iter()
            .copied()
            .filter(|number| secrets.iter().all(|(target, _)| target != number)),
    )?;
    let mut pipes = Vec::with_capacity(secrets.len());
    for (target, secret) in secrets {
        let (reader, writer) =
            io::pipe().map_err(|error| SecretFdError::Setup(Cause::of(&error)))?;
        pipes.push(Pipe {
            target,
            reader,
            writer,
            secret,
        });
    }
    spawn(cmd, pipes, close_in_child)
}

struct Pipe {
    target: u32,
    reader: PipeReader,
    writer: PipeWriter,
    secret: Zeroizing<Vec<u8>>,
}

/// The highest target, and the lowest number a copy of a read end may take.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    highest: c_int,
    copies_from: c_int,
}

fn bounds(targets: impl IntoIterator<Item = u32>) -> Result<Bounds, SecretFdError> {
    let mut seen: Vec<c_int> = Vec::new();
    let mut highest = LOWEST_TARGET - 1;
    let mut highest_given = 0;
    for given in targets {
        let target = c_int::try_from(given).map_err(|_| SecretFdError::AboveLimit(given))?;
        if target < LOWEST_TARGET {
            return Err(SecretFdError::BelowThree(given));
        }
        if target > HIGHEST_TARGET {
            return Err(SecretFdError::AboveLimit(given));
        }
        if seen.contains(&target) {
            return Err(SecretFdError::Duplicate(given));
        }
        seen.push(target);
        if target > highest {
            highest = target;
            highest_given = given;
        }
    }
    let copies_from = highest
        .checked_add(1)
        .ok_or(SecretFdError::AboveLimit(highest_given))?;
    Ok(Bounds {
        highest,
        copies_from,
    })
}

/// The numbers the program must not get, as step 1 says: converted with
/// `try_from` and 3 or above, since the child must not close one of the
/// program's standard streams. A number too large for a `c_int` cannot be a
/// descriptor and is refused as a bad one.
fn closable(close_in_child: &[u32]) -> Result<Vec<c_int>, SecretFdError> {
    close_in_child
        .iter()
        .map(|&given| {
            let number = c_int::try_from(given).map_err(|_| {
                SecretFdError::Setup(Cause::of(&io::Error::from_raw_os_error(libc::EBADF)))
            })?;
            if number < LOWEST_TARGET {
                return Err(SecretFdError::BelowThree(given));
            }
            Ok(number)
        })
        .collect()
}

fn spawn(
    mut cmd: Command,
    pipes: Vec<Pipe>,
    close_in_child: &[u32],
) -> Result<RunningChild, SecretFdError> {
    let bounds = bounds(pipes.iter().map(|pipe| pipe.target))?;
    let close_in_child = closable(close_in_child)?;

    // Step 3: every read end moves above the targets.
    let mut copies: Vec<(OwnedFd, c_int)> = Vec::with_capacity(pipes.len());
    let mut writers = Vec::with_capacity(pipes.len());
    for pipe in pipes {
        let copy = copy_above(pipe.reader.as_fd(), bounds.copies_from)
            .map_err(|error| SecretFdError::Setup(Cause::of(&error)))?;
        drop(pipe.reader);
        let target =
            c_int::try_from(pipe.target).map_err(|_| SecretFdError::AboveLimit(pipe.target))?;
        copies.push((copy, target));
        writers.push((pipe.target, pipe.writer, pipe.secret));
    }

    // Step 4.
    let held =
        hold_up_to(bounds.highest).map_err(|error| SecretFdError::Setup(Cause::of(&error)))?;

    // Step 5. Only numbers go into the closure; the vectors are built here,
    // in this process, and the child only reads them.
    let pairs: Vec<(c_int, c_int)> = copies
        .iter()
        .map(|(copy, target)| (copy.as_raw_fd(), *target))
        .collect();
    // A number that is also a target holds the program's pipe once the
    // `dup2` loop has run, so it stays.
    let withheld: Vec<c_int> = close_in_child
        .into_iter()
        .filter(|number| pairs.iter().all(|&(_, target)| target != *number))
        .collect();
    // SAFETY: `pre_exec` runs the closure in the child after fork, where only
    // async-signal-safe calls are allowed. The closure reads `pairs` and
    // `withheld`, which were allocated before the fork and are not changed,
    // and calls `dup2` and `close` on numbers from them, and
    // `io::Error::last_os_error` when `dup2` fails; none of them allocates or
    // takes a lock. Each copy is open in the parent until the spawn has
    // returned, so it is open in the child, and each target is a valid
    // descriptor number from 3 to 255. `close` never touches a target, a
    // standard stream or a descriptor of `Command`'s own. It closes either a
    // copy, which is above every target and open in the parent alongside
    // `Command`'s descriptors, so it has a number of its own; or a number
    // from `withheld`, which is 3 or above and no target, and which the
    // caller keeps open in the parent until the spawn has returned, so no
    // copy, no held `/dev/null` descriptor and no descriptor of `Command`'s
    // own can have that number: in the child it is the descriptor this
    // process inherited.
    unsafe {
        cmd.pre_exec(move || {
            for &(copy, target) in &pairs {
                if libc::dup2(copy, target) == -1 {
                    return Err(io::Error::last_os_error());
                }
                if copy != target {
                    libc::close(copy);
                }
            }
            for &number in &withheld {
                libc::close(number);
            }
            Ok(())
        });
    }
    let spawned = cmd.spawn();

    // Step 6, before anything can fail or wait.
    drop(copies);
    drop(held);
    let child = spawned.map_err(|error| SecretFdError::Spawn(Cause::of(&error)))?;

    // Step 7.
    let deliveries = writers
        .into_iter()
        .map(|(target, writer, secret)| deliver(target, writer, secret))
        .collect();
    Ok(RunningChild { child, deliveries })
}

/// Step 3: a copy of `fd` at `lowest` or above, close-on-exec.
fn copy_above(fd: BorrowedFd<'_>, lowest: c_int) -> io::Result<OwnedFd> {
    // SAFETY: `fd` is borrowed from a read end this module owns, so it stays
    // open for the call. `F_DUPFD_CLOEXEC` takes no pointer and makes a new
    // descriptor that nothing else in this process knows. On success
    // `from_raw_fd` makes the returned `OwnedFd` its only owner, which closes
    // it once.
    unsafe {
        let raw = libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, lowest);
        if raw == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(OwnedFd::from_raw_fd(raw))
    }
}

/// Step 4: `/dev/null` on every number up to `highest` that is still free.
/// Each open takes the lowest free number, so the first one above `highest`
/// means none below it is left.
fn hold_up_to(highest: c_int) -> io::Result<Vec<File>> {
    let mut held = Vec::new();
    loop {
        let file = File::open("/dev/null")?;
        if file.as_raw_fd() > highest {
            return Ok(held);
        }
        held.push(file);
    }
}

/// Step 7 for one pipe. If the thread cannot start, the writer and the secret
/// are dropped with the closure: the program reads end of file at once and
/// the secret is wiped.
fn deliver(target: u32, writer: PipeWriter, secret: Zeroizing<Vec<u8>>) -> Delivery {
    let len = secret.len();
    match thread::Builder::new().spawn(move || write_secret(writer, &secret, target)) {
        Ok(thread) => Delivery::Writing {
            target,
            len,
            thread,
        },
        Err(error) => Delivery::Failed(SecretFdError::Write {
            target,
            written: 0,
            len,
            cause: Cause::of(&error),
        }),
    }
}

/// Writes all of `secret` and drops the writer, which closes it. A failure
/// reports how far it got, as numbers.
fn write_secret(mut writer: impl Write, secret: &[u8], target: u32) -> Result<(), SecretFdError> {
    let mut written = 0;
    while written < secret.len() {
        match writer.write(&secret[written..]) {
            Ok(0) => {
                return Err(SecretFdError::Write {
                    target,
                    written,
                    len: secret.len(),
                    cause: Cause {
                        kind: io::ErrorKind::WriteZero,
                        code: None,
                    },
                });
            }
            Ok(n) => written += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(SecretFdError::Write {
                    target,
                    written,
                    len: secret.len(),
                    cause: Cause::of(&error),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Seek};
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::Duration;

    /// This file, for the tests that read it.
    const SOURCE: &str = include_str!("secret_fd.rs");

    /// The tests that would hang on a broken rule give up after this long.
    /// The programs they start take a second at most; the rest is room for a
    /// busy machine.
    const TIME_LIMIT: Duration = Duration::from_secs(30);

    /// No path on this Mac or a CI runner.
    const MISSING_PROGRAM: &str = "/nonexistent/ferrispass-secret-fd-test";

    /// What the leak tests look for, in full and in every part of four
    /// characters.
    const SENTINEL: &str = "sfd-sentinel-9Qz!2026";

    fn secret(text: &str) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(text.as_bytes().to_vec())
    }

    /// `/bin/sh -c <script>`, with its standard output in a file the test
    /// reads afterwards.
    fn sh(script: &str) -> (Command, File) {
        let out = tempfile::tempfile().expect("a file for the program's output");
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(out.try_clone().expect("a second handle on the output file"));
        (cmd, out)
    }

    fn read_back(mut out: File) -> String {
        let mut text = String::new();
        out.rewind().expect("rewind the output file");
        out.read_to_string(&mut text).expect("read the output file");
        text
    }

    /// Runs `f` on a thread of its own and fails the test if it takes longer
    /// than [`TIME_LIMIT`].
    fn within_time_limit<T: Send + 'static>(
        what: &str,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (done, outcome) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(f());
        });
        match outcome.recv_timeout(TIME_LIMIT) {
            Ok(value) => value,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                panic!("{what} did not return within {TIME_LIMIT:?}")
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("{what} panicked"),
        }
    }

    /// Waits for the program within the time limit. On a timeout the program
    /// is killed first, so a test that fails leaves nothing running.
    fn finish(running: RunningChild) -> Finished {
        let pid = running.id();
        let (done, outcome) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(running.wait());
        });
        match outcome.recv_timeout(TIME_LIMIT) {
            Ok(finished) => finished.expect("wait for the program"),
            Err(_) => {
                let _ = Command::new("/bin/kill")
                    .arg("-9")
                    .arg(pid.to_string())
                    .status();
                panic!("waiting for the program did not return within {TIME_LIMIT:?}");
            }
        }
    }

    /// Starts `script` with `secrets` and returns its output once it and the
    /// writers are done.
    fn run(script: &str, secrets: Vec<(u32, Zeroizing<Vec<u8>>)>) -> (Finished, String) {
        let (cmd, out) = sh(script);
        let running = spawn_with_secret_fds(cmd, secrets, &[]).expect("start the program");
        let finished = finish(running);
        (finished, read_back(out))
    }

    /// A pipe the test made itself, for the tests that need to know the
    /// numbers its ends got.
    fn own_pipe(text: &str) -> (Pipe, c_int) {
        let (reader, writer) = io::pipe().expect("a pipe");
        let number = reader.as_raw_fd();
        let target = u32::try_from(number).expect("a descriptor number");
        assert!(
            (3..=255).contains(&number),
            "the test process already holds so many descriptors that the pipe got {number}"
        );
        (
            Pipe {
                target,
                reader,
                writer,
                secret: secret(text),
            },
            number,
        )
    }

    /// Whether `text` holds `SENTINEL` or any four characters in a row of it.
    fn holds_part_of_sentinel(text: &str) -> Option<&'static str> {
        (0..=SENTINEL.len() - 4)
            .map(|from| &SENTINEL[from..from + 4])
            .find(|part| text.contains(part))
    }

    #[test]
    fn child_reads_the_secret_from_the_descriptor() {
        let (finished, out) = run("cat <&3", vec![(3, secret("hunter2"))]);
        assert!(finished.status.success(), "{finished:?}");
        assert_eq!(out, "hunter2");
        assert!(finished.undelivered.is_empty(), "{finished:?}");
    }

    #[test]
    fn two_descriptors_two_secrets() {
        let (finished, out) = run(
            "cat <&3; printf '|'; cat <&4",
            vec![(3, secret("first")), (4, secret("second"))],
        );
        assert!(finished.status.success(), "{finished:?}");
        assert_eq!(out, "first|second");
    }

    #[test]
    fn descriptors_below_three_are_refused() {
        for target in [0, 1, 2] {
            let (cmd, out) = sh("echo started");
            let refused = spawn_with_secret_fds(cmd, vec![(target, secret("x"))], &[]).err();
            assert_eq!(refused, Some(SecretFdError::BelowThree(target)));
            assert_eq!(read_back(out), "", "the program must not start");
            assert_eq!(
                check_targets([3, target]),
                Err(SecretFdError::BelowThree(target))
            );
        }
    }

    #[test]
    fn duplicate_target_descriptor_is_refused() {
        let (cmd, out) = sh("echo started");
        let refused =
            spawn_with_secret_fds(cmd, vec![(3, secret("a")), (3, secret("b"))], &[]).err();
        assert_eq!(refused, Some(SecretFdError::Duplicate(3)));
        assert_eq!(read_back(out), "", "the program must not start");
    }

    #[test]
    fn targets_above_255_are_refused() {
        assert_eq!(check_targets([255]), Ok(()));
        for target in [256, 1 << 31, u32::MAX] {
            assert_eq!(
                check_targets([target]),
                Err(SecretFdError::AboveLimit(target))
            );
        }
    }

    /// `cat` ends only at end of file, which it sees once every write end is
    /// closed. A write end left open hangs it, and the time limit fails the
    /// test.
    #[test]
    fn write_end_is_closed_after_the_secret() {
        let (finished, out) = run("cat <&3; printf '|end'", vec![(3, secret("closed after"))]);
        assert!(finished.status.success(), "{finished:?}");
        assert_eq!(out, "closed after|end");
    }

    /// The read end of a pipe often gets the very number the program wants
    /// it on: in the CLI, the pipe for target 3 can get descriptor 3. In this
    /// test process the free numbers depend on the other tests, so the test
    /// makes the pipe first and asks for whatever number its read end got.
    #[test]
    fn target_fd_3_works_when_the_pipe_got_fd_3() {
        let (pipe, number) = own_pipe("on its own number");
        let (cmd, out) = sh(&format!("cat <&{number}"));
        let running = spawn(cmd, vec![pipe], &[]).expect("start the program");
        let finished = finish(running);
        assert!(finished.status.success(), "{finished:?}");
        assert_eq!(read_back(out), "on its own number");
    }

    #[test]
    fn two_pipes_land_on_3_and_4_in_either_order() {
        let (finished, out) = run(
            "cat <&3; printf '|'; cat <&4",
            vec![(4, secret("four")), (3, secret("three"))],
        );
        assert!(finished.status.success(), "{finished:?}");
        assert_eq!(out, "three|four");

        // Crossed: each pipe goes to the number the other one's read end
        // got, so moving one in place would overwrite the other.
        let (mut first, first_number) = own_pipe("first");
        let (mut second, second_number) = own_pipe("second");
        first.target = u32::try_from(second_number).expect("a descriptor number");
        second.target = u32::try_from(first_number).expect("a descriptor number");
        let (cmd, out) = sh(&format!(
            "cat <&{second_number}; printf '|'; cat <&{first_number}"
        ));
        let running = spawn(cmd, vec![first, second], &[]).expect("start the program");
        let finished = finish(running);
        assert!(finished.status.success(), "{finished:?}");
        assert_eq!(read_back(out), "first|second");
    }

    /// 200 KiB is more than a pipe holds. Written before the spawn, the
    /// writer would block with no reader; the program here also waits a
    /// second before it reads.
    #[test]
    fn a_secret_larger_than_the_pipe_buffer_arrives_complete() {
        let big = Zeroizing::new(vec![b'x'; 200 * 1024]);
        let (finished, out) = within_time_limit("start, write and wait", move || {
            run("sleep 1; wc -c <&3", vec![(3, big)])
        });
        assert!(finished.status.success(), "{finished:?}");
        assert_eq!(out.trim(), "204800");
        assert!(finished.undelivered.is_empty(), "{finished:?}");
    }

    /// The program exits without reading. The writer gets `EPIPE` only if no
    /// read end is left open anywhere, this process included.
    #[test]
    fn run_returns_when_the_child_exits_without_reading() {
        let big = Zeroizing::new(vec![b'x'; 200 * 1024]);
        let finished = within_time_limit("start, write and wait", move || {
            run("exit 0", vec![(3, big)]).0
        });
        assert!(finished.status.success(), "{finished:?}");
        assert!(
            matches!(
                finished.undelivered.as_slice(),
                [SecretFdError::Write {
                    target: 3,
                    cause: Cause {
                        kind: io::ErrorKind::BrokenPipe,
                        ..
                    },
                    ..
                }]
            ),
            "{finished:?}"
        );
    }

    /// A program that does not exist. Afterwards no read end of the first
    /// pipe may be open (a write to it fails with `EPIPE`), and no write end
    /// of the second (a read from it gets end of file). Watching the pipes
    /// tests every copy, wherever it went, and does not depend on which
    /// numbers the other tests in this process take in the meantime.
    ///
    /// A program another test starts while `std::io::pipe` is between `pipe`
    /// and setting close-on-exec keeps both ends until it exits, which takes
    /// a second at most. So the test asks again for a while before it calls
    /// an end open for good.
    #[test]
    fn spawn_failure_closes_every_descriptor() {
        let (mut first, _) = own_pipe("first");
        let (mut second, _) = own_pipe("second");
        let mut first_writer = first.writer.try_clone().expect("a second write end");
        let mut second_reader = second.reader.try_clone().expect("a second read end");
        first.target = 40;
        second.target = 41;

        let refused = spawn(Command::new(MISSING_PROGRAM), vec![first, second], &[]).err();
        assert!(
            matches!(
                refused,
                Some(SecretFdError::Spawn(Cause {
                    kind: io::ErrorKind::NotFound,
                    ..
                }))
            ),
            "{refused:?}"
        );

        // One byte at a time, so the pipe never fills and a write never
        // blocks while someone else still holds a read end.
        let read_ends_closed = within_time_limit("writing the first pipe", move || {
            for _ in 0..100 {
                match first_writer.write_all(b"x") {
                    Err(error) => return error.kind() == io::ErrorKind::BrokenPipe,
                    Ok(()) => thread::sleep(Duration::from_millis(100)),
                }
            }
            false
        });
        assert!(
            read_ends_closed,
            "a read end of the first pipe is still open"
        );
        let read = within_time_limit("reading the second pipe", move || {
            let mut buf = [0; 16];
            second_reader.read(&mut buf).map_err(|error| error.kind())
        });
        assert_eq!(read, Ok(0), "a write end of the second pipe is still open");
    }

    /// `Command::spawn` learns about a failed exec on descriptors of its own.
    /// Wherever the target is, those must not be among the numbers the child
    /// replaces, or a missing program would look started.
    #[test]
    fn a_failed_exec_is_reported_whatever_the_target() {
        for target in 3..=24 {
            match spawn_with_secret_fds(
                Command::new(MISSING_PROGRAM),
                vec![(target, secret("x"))],
                &[],
            ) {
                Err(SecretFdError::Spawn(cause)) => {
                    assert_eq!(cause.kind, io::ErrorKind::NotFound, "target {target}")
                }
                Err(other) => panic!("target {target}: {other}"),
                Ok(running) => {
                    let finished = finish(running);
                    panic!("target {target}: a missing program looked started, {finished:?}");
                }
            }
        }
    }

    #[test]
    fn secret_is_not_in_argv_or_environment() {
        let (mut cmd, out) = sh("env; echo \"$0\" \"$@\"");
        cmd.arg("program-name").arg("first-argument");
        let running = spawn_with_secret_fds(cmd, vec![(3, secret("only-on-the-descriptor"))], &[])
            .expect("start the program");
        let finished = finish(running);
        assert!(finished.status.success(), "{finished:?}");
        let out = read_back(out);
        assert!(
            out.contains("program-name first-argument"),
            "the program printed its arguments: {out}"
        );
        assert!(
            !out.contains("only-on-the-descriptor"),
            "the secret is in the program's arguments or environment: {out}"
        );
    }

    /// A number the program must not get that is also a target: the child
    /// leaves it alone after `dup2` has put the pipe there, so the program
    /// still reads its secret on it. The close of a number that is not a
    /// target needs a descriptor without close-on-exec in this process, and
    /// the standard library opens none; the shell gives the CLI one in
    /// `run_keeps_the_master_password_descriptor_from_the_program`.
    #[test]
    fn a_number_to_close_that_is_a_target_keeps_its_secret() {
        let (cmd, out) = sh("cat <&3; printf '|'; cat <&4");
        let running =
            spawn_with_secret_fds(cmd, vec![(3, secret("three")), (4, secret("four"))], &[4])
                .expect("start the program");
        let finished = finish(running);
        assert!(finished.status.success(), "{finished:?}");
        assert_eq!(read_back(out), "three|four");
    }

    #[test]
    fn numbers_to_close_below_three_or_past_any_descriptor_are_refused() {
        for number in [0, 1, 2] {
            let (cmd, out) = sh("echo started");
            let refused = spawn_with_secret_fds(cmd, vec![(3, secret("x"))], &[number]).err();
            assert_eq!(refused, Some(SecretFdError::BelowThree(number)));
            assert_eq!(read_back(out), "", "the program must not start");
        }
        for number in [1 << 31, u32::MAX] {
            let (cmd, out) = sh("echo started");
            let refused = spawn_with_secret_fds(cmd, vec![(3, secret("x"))], &[number]).err();
            assert!(
                matches!(
                    refused,
                    Some(SecretFdError::Setup(Cause {
                        code: Some(libc::EBADF),
                        ..
                    }))
                ),
                "{number}: {refused:?}"
            );
            assert_eq!(read_back(out), "", "the program must not start");
        }
    }

    /// A number to close has to be open here when the spawn starts. If it
    /// were not, a pipe or a copy could get it, and the child would close
    /// that instead. No process holds a million descriptors, so 1000000 is
    /// closed in this one whatever the other tests open.
    #[test]
    fn a_closed_number_to_close_is_refused_before_touch_id() {
        const CLOSED: u32 = 1_000_000;
        assert_eq!(check_open([CLOSED]), Err(SecretFdError::NotOpen(CLOSED)));

        let (cmd, out) = sh("echo started");
        let refused = spawn_with_secret_fds(cmd, vec![(3, secret("x"))], &[CLOSED]).err();
        assert_eq!(refused, Some(SecretFdError::NotOpen(CLOSED)));
        assert_eq!(read_back(out), "", "the program must not start");

        // One this test holds open passes. A number that is also a target is
        // not asked about, open here or not: `dup2` puts the program's pipe
        // there.
        let held = File::open("/dev/null").expect("open /dev/null");
        let number = u32::try_from(held.as_raw_fd()).expect("a descriptor number");
        assert_eq!(check_open([number]), Ok(()));
        let (cmd, out) = sh("cat <&3");
        let running = spawn_with_secret_fds(cmd, vec![(3, secret("on 3"))], &[number, 3])
            .expect("start the program");
        let finished = finish(running);
        assert!(finished.status.success(), "{finished:?}");
        assert_eq!(read_back(out), "on 3");
    }

    /// A writer that takes `accept` bytes and then fails with an error whose
    /// message is the sentinel.
    struct FailingWriter {
        accept: usize,
    }

    impl Write for FailingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.accept == 0 {
                return Err(io::Error::other(SENTINEL));
            }
            let n = buf.len().min(self.accept);
            self.accept -= n;
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn errors_display_and_debug_hold_no_secret() {
        let with_text = || io::Error::other(SENTINEL);
        // Not `NotFound`: its text, "entity not found", holds "enti", four
        // characters of the sentinel, and says nothing about a secret.
        let with_path =
            || io::Error::new(io::ErrorKind::PermissionDenied, format!("/tmp/{SENTINEL}"));
        let errors = [
            SecretFdError::Setup(Cause::of(&with_text())),
            SecretFdError::Spawn(Cause::of(&with_path())),
            SecretFdError::Wait(Cause::of(&with_text())),
            write_secret(FailingWriter { accept: 4 }, SENTINEL.as_bytes(), 3)
                .expect_err("the writer fails"),
            write_secret(FailingWriter { accept: 0 }, SENTINEL.as_bytes(), 4)
                .expect_err("the writer fails"),
            SecretFdError::BelowThree(2),
            SecretFdError::AboveLimit(256),
            SecretFdError::Duplicate(3),
            SecretFdError::NotOpen(9),
        ];
        for error in &errors {
            let mut texts = vec![format!("{error}"), format!("{error:?}")];
            let mut source = std::error::Error::source(error);
            while let Some(inner) = source {
                texts.push(format!("{inner}"));
                texts.push(format!("{inner:?}"));
                source = inner.source();
            }
            for text in texts {
                assert_eq!(holds_part_of_sentinel(&text), None, "{text}");
            }
        }
        assert_eq!(
            errors[3],
            SecretFdError::Write {
                target: 3,
                written: 4,
                len: SENTINEL.len(),
                cause: Cause {
                    kind: io::ErrorKind::Other,
                    code: None
                },
            }
        );
    }

    /// The program exits without reading a secret larger than the pipe
    /// buffer. What comes back names the descriptor and counts bytes, and
    /// holds no part of the secret.
    #[test]
    fn a_write_failure_reports_no_bytes() {
        let big = Zeroizing::new(SENTINEL.repeat(10_000).into_bytes());
        let len = big.len();
        let finished = within_time_limit("start, write and wait", move || {
            run("exit 0", vec![(3, big)]).0
        });
        let [error] = finished.undelivered.as_slice() else {
            panic!("one undelivered secret: {finished:?}");
        };
        let SecretFdError::Write {
            target, written, ..
        } = error
        else {
            panic!("a write error: {error:?}");
        };
        assert_eq!(*target, 3);
        assert!(*written < len, "{error:?}");
        let shown = format!("{error}");
        assert!(shown.contains(&format!("of {len} bytes")), "{shown}");
        for text in [shown, format!("{error:?}")] {
            assert_eq!(holds_part_of_sentinel(&text), None, "{text}");
        }
    }

    /// The code of this file without its tests.
    fn production() -> &'static str {
        SOURCE
            .split("\n#[cfg(test)]\nmod tests {")
            .next()
            .expect("the file has code before its tests")
    }

    /// The names called in `tokens`: a name followed by `(`.
    fn calls<'s>(tokens: &[Tok<'s>]) -> Vec<&'s str> {
        tokens
            .windows(2)
            .filter_map(|pair| match pair {
                [Tok::Ident(name), Tok::Punct('(')] => Some(*name),
                _ => None,
            })
            .collect()
    }

    /// The macros invoked in `tokens`, each with the tokens of its
    /// arguments.
    fn macros<'s>(tokens: &[Tok<'s>]) -> Vec<(&'s str, Vec<Tok<'s>>)> {
        let mut found = Vec::new();
        for i in 0..tokens.len().saturating_sub(2) {
            if let [Tok::Ident(name), Tok::Punct('!'), Tok::Punct(open)] = tokens[i..i + 3]
                && matches!(open, '(' | '[' | '{')
            {
                let end = closing(tokens, i + 2);
                found.push((name, tokens[i + 3..end].to_vec()));
            }
        }
        found
    }

    /// The index of the bracket that closes the one at `open`.
    fn closing(tokens: &[Tok<'_>], open: usize) -> usize {
        let mut depth = 0;
        for (i, token) in tokens.iter().enumerate().skip(open) {
            match token {
                Tok::Punct('(' | '[' | '{') => depth += 1,
                Tok::Punct(')' | ']' | '}') => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            }
        }
        panic!("a bracket that never closes");
    }

    /// The child runs the `pre_exec` closure between fork and exec, where a
    /// lock another thread held at the fork stays held for good and an
    /// allocation can deadlock. The closure may only call `dup2`, `close`
    /// and `io::Error::last_os_error`, compare, and return.
    #[test]
    fn pre_exec_only_duplicates_and_closes() {
        let tokens = code_tokens(production());
        let starts: Vec<usize> = (0..tokens.len().saturating_sub(2))
            .filter(|&i| {
                tokens[i..i + 3] == [Tok::Punct('.'), Tok::Ident("pre_exec"), Tok::Punct('(')]
            })
            .collect();
        let [start] = starts.as_slice() else {
            panic!("one call of pre_exec, found {}", starts.len());
        };
        let open = start + 2;
        let closure = &tokens[open + 1..closing(&tokens, open)];

        let called = calls(closure);
        for name in &called {
            assert!(
                ["dup2", "close", "last_os_error", "Ok", "Err"].contains(name),
                "the pre_exec closure calls `{name}`"
            );
        }
        for name in ["dup2", "close"] {
            assert!(
                called.contains(&name),
                "the scan no longer sees `{name}` in the pre_exec closure: {called:?}"
            );
        }
        let invoked: Vec<&str> = macros(closure).into_iter().map(|(name, _)| name).collect();
        assert!(
            invoked.is_empty(),
            "macros in the pre_exec closure: {invoked:?}"
        );
        for name in [
            "format",
            "String",
            "Vec",
            "Box",
            "Rc",
            "Arc",
            "Mutex",
            "RwLock",
            "lock",
            "println",
            "eprintln",
            "print",
            "eprint",
            "dbg",
            "panic",
            "unwrap",
            "expect",
            "to_string",
            "to_owned",
            "clone",
            "collect",
            "push",
        ] {
            assert!(
                !closure.contains(&Tok::Ident(name)),
                "`{name}` in the pre_exec closure"
            );
        }
    }

    /// The header says how many `unsafe` blocks there are and which `libc`
    /// functions they call. The file has that many, and calls exactly the
    /// `libc` functions the header names, no other and none fewer.
    #[test]
    fn secret_fd_has_exactly_the_declared_unsafe_sites() {
        let header: String = SOURCE
            .lines()
            .take_while(|line| line.starts_with("//!"))
            .map(|line| line.trim_start_matches("//!").trim())
            .collect::<Vec<_>>()
            .join(" ");
        let declared = header
            .split("There are exactly ")
            .nth(1)
            .and_then(|rest| rest.split_once(" `unsafe` blocks in this file"))
            .map(|(count, _)| count)
            .expect("the header says how many `unsafe` blocks there are");
        let declared = ["one", "two", "three", "four", "five"]
            .iter()
            .position(|word| *word == declared)
            .map(|at| at + 1)
            .unwrap_or_else(|| panic!("`{declared}` is not a number word"));
        let named: Vec<&str> = header
            .split("the `libc` calls in them are ")
            .nth(1)
            .and_then(|rest| rest.split_once(':'))
            .map(|(list, _)| list.split('`').skip(1).step_by(2).collect())
            .expect("the header names the `libc` calls");

        let tokens = code_tokens(SOURCE);
        let found = tokens
            .iter()
            .filter(|token| **token == Tok::Ident("unsafe"))
            .count();
        assert_eq!(
            found, declared,
            "the header declares {declared} `unsafe` blocks, the file has {found}"
        );

        let libc_calls: Vec<&str> = tokens
            .windows(5)
            .filter_map(|window| match window {
                [
                    Tok::Ident("libc"),
                    Tok::Punct(':'),
                    Tok::Punct(':'),
                    Tok::Ident(name),
                    Tok::Punct('('),
                ] => Some(*name),
                _ => None,
            })
            .collect();
        for name in &libc_calls {
            assert!(
                named.contains(name),
                "the file calls libc::{name}, which the header does not name: {named:?}"
            );
        }
        for name in &named {
            assert!(
                libc_calls.contains(name),
                "the header names libc::{name}, which the scan does not see: {libc_calls:?}"
            );
        }
    }

    /// No line of this module's code logs or prints, and no macro gets a
    /// secret buffer by its name, `secret` or `secrets`: neither as an
    /// argument, `format!("{:?}", secret)`, nor captured in its format string,
    /// `format!("{secret:?}")`. The scan goes by those names. The same bytes
    /// under another name are left to `errors_display_and_debug_hold_no_secret`
    /// and the process tests, which look at what comes out.
    #[test]
    fn secret_fd_never_logs_the_secret() {
        let tokens = code_tokens(production());
        let leaks = leaks(&tokens);
        assert!(
            leaks.is_empty(),
            "the secret could reach a log or a message: {leaks:?}"
        );
        // The scan has to see the buffer it guards, or it proves nothing.
        assert!(
            tokens.contains(&Tok::Ident("secret")),
            "the scan no longer sees `secret` in this file's code"
        );
    }

    /// What `secret_fd_never_logs_the_secret` reports in `tokens`: every
    /// printing macro, every `log::` or `tracing::` path, and every macro that
    /// gets a buffer by name.
    fn leaks(tokens: &[Tok<'_>]) -> Vec<String> {
        let mut leaks = Vec::new();
        for pair in tokens.windows(2) {
            match pair {
                [Tok::Ident(name), Tok::Punct('!')]
                    if ["println", "eprintln", "print", "eprint", "dbg"].contains(name) =>
                {
                    leaks.push(format!("{name}!"));
                }
                [Tok::Ident(name @ ("log" | "tracing")), Tok::Punct(':')] => {
                    leaks.push(format!("{name}::"));
                }
                _ => {}
            }
        }
        for (name, arguments) in macros(tokens) {
            for buffer in ["secret", "secrets"] {
                let passed = arguments.contains(&Tok::Ident(buffer));
                let captured = arguments.iter().any(|token| match token {
                    Tok::Str(text) => captures(text).contains(&buffer),
                    _ => false,
                });
                if passed || captured {
                    leaks.push(format!("{name}! with `{buffer}`"));
                }
            }
        }
        leaks
    }

    /// The names a format string reads in its braces: `secret` in
    /// `{secret}`, `{secret:?}` and `{0:>secret$}`, but none in
    /// `{{secret}}`, which prints the braces and the word. It does not ask
    /// whether `text` is the format string: in any string a macro gets,
    /// `{secret}` counts.
    fn captures(text: &str) -> Vec<&str> {
        let mut names = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find(['{', '}']) {
            let brace = &rest[at..=at];
            let after = &rest[at + 1..];
            if after.starts_with(brace) {
                rest = &after[1..];
            } else if brace == "}" {
                rest = after;
            } else {
                let end = after.find('}').unwrap_or(after.len());
                names.extend(
                    after[..end]
                        .split(|c: char| c != '_' && !c.is_ascii_alphanumeric())
                        .filter(|name| !name.is_empty()),
                );
                rest = after.get(end + 1..).unwrap_or("");
            }
        }
        names
    }

    /// The leak scan on code it has to report and code it has to let
    /// through.
    #[test]
    fn the_leak_scan_sees_a_buffer_captured_in_a_format_string() {
        for reported in [
            r#"format!("could not write {secret}")"#,
            r#"format!("could not write {secret:?}")"#,
            r#"format!("{} of {secrets:?}", len)"#,
            r#"write!(f, "{0:>secret$}", len)"#,
            r#"panic!("{{{secret}}}")"#,
            r#"format!("could not write {:?}", secret)"#,
        ] {
            assert_eq!(
                leaks(&code_tokens(reported)).len(),
                1,
                "the scan lets `{reported}` through"
            );
        }
        for clean in [
            r#"format!("{{secret}}")"#,
            r#"format!("{secret_len} of {len}")"#,
            r#"format!("the secret is {}", len)"#,
        ] {
            assert_eq!(
                leaks(&code_tokens(clean)),
                Vec::<String>::new(),
                "the scan reports `{clean}`"
            );
        }
    }

    #[test]
    fn the_scan_skips_comments_and_strings() {
        let tokens = code_tokens(
            "// unsafe\n/* unsafe /* nested */ unsafe */ \"unsafe\\\" { \" r#\"unsafe\"# 'x' \
             b'\\'' fn f<'a>(x: &'a u8) { g(x); println!(\"{}\", secret); }",
        );
        assert!(!tokens.contains(&Tok::Ident("unsafe")), "{tokens:?}");
        assert_eq!(tokens.first(), Some(&Tok::Str("unsafe\\\" { ")));
        assert_eq!(calls(&tokens), ["g"]);
        let found = macros(&tokens);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "println");
        assert!(found[0].1.contains(&Tok::Ident("secret")));
    }

    /// A token of code, for the scans above: a name, a string's contents, a
    /// single punctuation character, or anything else as written (numbers,
    /// characters, lifetimes). Comments are dropped, so a word in one is
    /// never code; a string becomes one token, so a word in one is never a
    /// name.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Tok<'s> {
        Ident(&'s str),
        Str(&'s str),
        Punct(char),
        Other(&'s str),
    }

    /// The tokens of `source`, after the tokenizer in `tests/architecture.rs`
    /// cut down to what this file contains.
    fn code_tokens(source: &str) -> Vec<Tok<'_>> {
        let bytes = source.as_bytes();
        let at = |i: usize| bytes.get(i).copied().unwrap_or(0);
        let ident = |c: u8| c == b'_' || c.is_ascii_alphanumeric();
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_whitespace() {
                i += 1;
            } else if c == b'/' && at(i + 1) == b'/' {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            } else if c == b'/' && at(i + 1) == b'*' {
                let mut depth = 0;
                loop {
                    match (at(i), at(i + 1)) {
                        (0, _) => panic!("a block comment that never ends"),
                        (b'/', b'*') => {
                            depth += 1;
                            i += 2;
                        }
                        (b'*', b'/') => {
                            depth -= 1;
                            i += 2;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => i += 1,
                    }
                }
            } else if c == b'r' && at(i + 1) == b'#' && at(i + 2) == b'"' {
                let from = i + 3;
                let end = source[from..]
                    .find("\"#")
                    .map(|n| from + n)
                    .expect("a raw string that never ends");
                tokens.push(Tok::Str(&source[from..end]));
                i = end + 2;
            } else if c == b'"' || (c == b'b' && at(i + 1) == b'"') {
                let from = i + if c == b'b' { 2 } else { 1 };
                i = from;
                while at(i) != b'"' {
                    assert!(i < bytes.len(), "a string that never ends");
                    i += if at(i) == b'\\' { 2 } else { 1 };
                }
                tokens.push(Tok::Str(&source[from..i]));
                i += 1;
            } else if c == b'\'' || (c == b'b' && at(i + 1) == b'\'') {
                let from = i;
                if c == b'b' {
                    i += 1;
                }
                if at(i + 1) == b'\\' {
                    i += 3;
                    while at(i) != b'\'' {
                        assert!(i < bytes.len(), "a character literal that never ends");
                        i += 1;
                    }
                    i += 1;
                } else if at(i + 2) == b'\'' {
                    i += 3;
                } else {
                    // A lifetime or a label.
                    i += 1;
                    while ident(at(i)) {
                        i += 1;
                    }
                }
                tokens.push(Tok::Other(&source[from..i]));
            } else if c == b'_' || c.is_ascii_alphabetic() {
                let from = i;
                while ident(at(i)) {
                    i += 1;
                }
                tokens.push(Tok::Ident(&source[from..i]));
            } else if c.is_ascii_digit() {
                let from = i;
                while ident(at(i)) || (at(i) == b'.' && at(i + 1).is_ascii_digit()) {
                    i += 1;
                }
                tokens.push(Tok::Other(&source[from..i]));
            } else if c.is_ascii() {
                tokens.push(Tok::Punct(c as char));
                i += 1;
            } else {
                // Outside comments and strings, this file is ASCII.
                let width = source[i..].chars().next().map_or(1, char::len_utf8);
                tokens.push(Tok::Other(&source[i..i + width]));
                i += width;
            }
        }
        tokens
    }
}
