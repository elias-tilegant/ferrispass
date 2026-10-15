//! Architecture rules checked by reading the source (redesign plan, part 20,
//! section 2.10; test T0.3).
//!
//! Rule 4: new `unsafe` goes only into `src/platform/`, one opt-in per file,
//! with the reason in the module header. `Cargo.toml` sets the lint
//! `unsafe_code = "deny"`, so the compiler already rejects `unsafe` wherever it
//! is not allowed. What it accepts without a second look is a new
//! `allow(unsafe_code)`: one attribute, and everything under it compiles. This
//! test holds the complete list of those attributes and fails on
//!
//! - any attribute that relaxes the lint (`allow`, `expect` or `warn`, also
//!   inside `cfg_attr`) and is not on the list, or a listed one that is gone,
//! - any unsafe construct outside the items the list covers, so the rule
//!   still holds if the lint level were ever lowered,
//! - a lint level weaker than `deny` in `Cargo.toml`, and lint flags in the
//!   Cargo configuration or the CI workflows,
//! - code the scan cannot see: `include!`, a `#[path]` module or a build
//!   target that is not a `.rs` file inside the repository.
//!
//! Every `.rs` file under the manifest directory is read, except `target/` and
//! `.git/`, so a module or target in an unexpected place is not missed. The
//! scan works on tokens, not lines. Comments, doc comments, string and
//! character literals are skipped, so a word in a message or a comment is not
//! a finding.
//!
//! What it cannot see is unsafe code that another crate's macro generates
//! without the word `unsafe` in this crate's source. rustc does not report
//! `unsafe_code` for code expanded from another crate's macro either; those
//! crates are reviewed with the supply chain (docs/release-process.md). A
//! `static mut` declaration passes both, but every access to it needs an
//! `unsafe` block, which does not. Flags from outside the repository (the
//! environment, a Cargo configuration above it) are out of reach; CI sets
//! none, and the workflow check below keeps it that way.

use std::fs;
use std::path::{Path, PathBuf};

/// Opt-ins that predate rule 4. The list only shrinks: a new opt-in is a file
/// under `src/platform/` and goes into [`PLATFORM_MODULES`]. Each entry names
/// the file and the item the attribute covers (`file` for a whole file), not
/// a line, so an unrelated edit above it does not break the test.
const LEGACY_EXCEPTIONS: &[(&str, &str)] = &[
    ("src/biometric/macos.rs", "file"),
    ("src/keepass/document.rs", "fn publish_new_no_clobber"),
    ("src/keepass/document.rs", "mod macos_full_sync"),
    ("src/launch/tempfile.rs", "fn process_uid"),
    ("src/session_lock.rs", "file"),
    ("src/sync/icloud.rs", "mod platform"),
    ("src/update/macos_installer.rs", "file"),
];

/// Audited platform modules (V8). Each is a file under `src/platform/` that
/// opts in once with `#![allow(unsafe_code)]` below a `//!` header that says
/// why it needs `unsafe`.
const PLATFORM_MODULES: &[&str] = &[];

/// Top-level directories that are never scanned: build output and Git's own
/// data. Everything else under the manifest directory is.
const SKIPPED_DIRS: &[&str] = &["target", ".git"];

#[test]
fn unsafe_code_stays_on_the_exception_list() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    let mut problems = Vec::new();
    let mut scanned = 0;

    for (rel, path) in rust_sources(root) {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
        let report = match scan(&source) {
            Ok(report) => report,
            Err(err) => panic!("cannot tokenize {rel}: {err}"),
        };
        scanned += 1;
        for exception in report.exceptions {
            found.push((rel.clone(), exception.anchor, exception.line));
        }
        for (line, problem) in report.problems {
            problems.push(format!("{rel}:{line}: {problem}"));
        }
    }
    assert!(
        scanned > 50,
        "only {scanned} Rust files found under {root:?}"
    );

    let mut expected: Vec<(String, String)> = LEGACY_EXCEPTIONS
        .iter()
        .map(|(file, anchor)| (file.to_string(), anchor.to_string()))
        .chain(
            PLATFORM_MODULES
                .iter()
                .map(|file| (file.to_string(), "file".to_string())),
        )
        .collect();
    expected.sort();
    let mut actual: Vec<(String, String)> = found
        .iter()
        .map(|(file, anchor, _)| (file.clone(), anchor.clone()))
        .collect();
    actual.sort();

    for (file, anchor, line) in &found {
        if !expected.contains(&(file.clone(), anchor.clone())) {
            problems.push(format!(
                "{file}:{line}: allows unsafe_code for `{anchor}`, which is not on the \
                 exception list. New unsafe belongs in its own module under src/platform/ \
                 with the reason in its header, listed in PLATFORM_MODULES."
            ));
        }
    }
    for (file, anchor) in &expected {
        if !actual.contains(&(file.clone(), anchor.clone())) {
            problems.push(format!(
                "{file}: the exception list names `{anchor}`, but no attribute there \
                 allows unsafe_code any more. Remove the entry."
            ));
        }
    }
    if actual.len() != expected.len() {
        problems.push(format!(
            "{} opt-ins found, {} listed; one item allows unsafe_code twice",
            actual.len(),
            expected.len()
        ));
    }

    for file in PLATFORM_MODULES {
        if !file.starts_with("src/platform/") {
            problems.push(format!("{file}: platform modules live under src/platform/"));
            continue;
        }
        match fs::read_to_string(root.join(file)) {
            Ok(source) if has_module_header(&source) => {}
            Ok(_) => problems.push(format!(
                "{file}: needs a //! header that says why it uses unsafe, above \
                 #![allow(unsafe_code)]"
            )),
            Err(err) => problems.push(format!("{file}: {err}")),
        }
    }

    assert!(
        problems.is_empty(),
        "unsafe outside the exception list (plan part 20, 2.10 rule 4):\n{}",
        problems.join("\n")
    );
}

#[test]
fn nothing_lowers_the_unsafe_code_lint() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    let level = unsafe_code_level(&manifest);
    assert!(
        matches!(level.as_deref(), Ok("deny" | "forbid")),
        "Cargo.toml must keep unsafe_code = \"deny\" under [lints.rust]; the exception \
         list relies on it. Found: {level:?}"
    );

    let mut problems = Vec::new();
    for name in [".cargo/config.toml", ".cargo/config"] {
        if let Ok(text) = fs::read_to_string(root.join(name)) {
            for word in cargo_config_overrides(&text) {
                problems.push(format!("{name}: {word}"));
            }
        }
    }
    let workflows = root.join(".github/workflows");
    if let Ok(entries) = fs::read_dir(&workflows) {
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if !path
                .extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
            {
                continue;
            }
            let text = fs::read_to_string(&path).expect("read workflow");
            let name = path.strip_prefix(root).unwrap_or(&path).display();
            for word in workflow_overrides(&text) {
                problems.push(format!("{name}: {word}"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "lint levels are set in Cargo.toml only, not through flags:\n{}",
        problems.join("\n")
    );
}

#[test]
fn build_targets_stay_where_the_scan_reads() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    let stray = stray_target_paths(&manifest);
    assert!(
        stray.is_empty(),
        "build targets must be .rs files inside the repository, or the unsafe scan \
         does not read them:\n{}",
        stray.join("\n")
    );
}

// ---------------------------------------------------------------- scanner --

fn rust_sources(root: &Path) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    let mut visited = Vec::new();
    collect_rust_files(root, root, &mut visited, &mut files);
    let mut sources: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|path| {
            let rel = path
                .strip_prefix(root)
                .expect("source under the manifest directory")
                .to_string_lossy()
                .replace('\\', "/");
            (rel, path)
        })
        .collect();
    sources.sort();
    sources
}

fn collect_rust_files(root: &Path, dir: &Path, visited: &mut Vec<PathBuf>, out: &mut Vec<PathBuf>) {
    // A symlinked directory is followed, as rustc follows it, but only once.
    let canonical = fs::canonicalize(dir)
        .unwrap_or_else(|err| panic!("cannot resolve {}: {err}", dir.display()));
    if visited.contains(&canonical) {
        return;
    }
    visited.push(canonical);
    let entries =
        fs::read_dir(dir).unwrap_or_else(|err| panic!("cannot list {}: {err}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            let skipped = dir == root
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| SKIPPED_DIRS.contains(&name));
            if !skipped {
                collect_rust_files(root, &path, visited, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The level `Cargo.toml` gives `unsafe_code`. An error for anything this
/// reader does not follow: no entry, or more than one (`unsafe-code` is the
/// same lint to rustc), a value that is not a level or `{ level = .. }`, or
/// lints set in `[lints]` itself rather than in `[lints.rust]`.
fn unsafe_code_level(manifest: &str) -> Result<String, String> {
    let mut table = String::new();
    let mut levels = Vec::new();
    for (index, raw) in manifest.lines().enumerate() {
        let line = strip_toml_comment(raw).trim();
        if line.starts_with('[') {
            table = toml_header(line);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if table == "[lints]" {
            return Err(format!(
                "line {}: lints set in [lints] itself, keep them in [lints.rust]",
                index + 1
            ));
        }
        if table != "[lints.rust]" || toml_key(key).replace('-', "_") != "unsafe_code" {
            continue;
        }
        let level = toml_level(value.trim())
            .ok_or_else(|| format!("line {}: cannot read the level in `{line}`", index + 1))?;
        levels.push(level);
    }
    match levels.as_slice() {
        [level] => Ok(level.clone()),
        [] => Err("no unsafe_code entry under [lints.rust]".to_string()),
        _ => Err(format!(
            "unsafe_code is set {} times under [lints.rust]",
            levels.len()
        )),
    }
}

/// `path` and `build` entries in `Cargo.toml` that point where the scan does
/// not read: absolute, through `..`, or not a `.rs` file. Dependency, patch
/// and replace tables are skipped; a path dependency is a crate of its own.
fn stray_target_paths(manifest: &str) -> Vec<String> {
    let mut table = String::new();
    let mut stray = Vec::new();
    for (index, raw) in manifest.lines().enumerate() {
        let line = strip_toml_comment(raw).trim();
        if line.starts_with('[') {
            table = toml_header(line);
            continue;
        }
        if ["dependencies", "patch", "replace"]
            .iter()
            .any(|word| table.contains(word))
        {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = toml_key(key);
        if key != "path" && key != "build" && !key.ends_with(".path") {
            continue;
        }
        // `build = false` and other non-strings name no file.
        let Some(target) = toml_string(value.trim()) else {
            continue;
        };
        if !stays_in_tree(&target) {
            stray.push(format!("Cargo.toml:{}: {key} = \"{target}\"", index + 1));
        }
    }
    stray
}

/// A relative path to a `.rs` file that does not climb out with `..`.
fn stays_in_tree(path: &str) -> bool {
    !path.starts_with('/') && !path.split('/').any(|part| part == "..") && path.ends_with(".rs")
}

/// Words in a Cargo configuration that change lint levels or pull in more
/// configuration: `-A unsafe_code`, `--cap-lints`, `--config`, `include`.
fn cargo_config_overrides(text: &str) -> Vec<String> {
    let text = decode_escapes(text).to_lowercase();
    let mut found: Vec<String> = ["unsafe_code", "unsafe-code", "cap-lints", "--config"]
        .into_iter()
        .filter(|word| text.contains(word))
        .map(str::to_string)
        .collect();
    let include_key = text.lines().any(|line| {
        line.trim_start()
            .strip_prefix("include")
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    });
    if include_key {
        found.push("include".to_string());
    }
    found
}

/// Words in a CI workflow that change lint levels: any rustflags variable or
/// setting (`RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS`, `build.rustflags`), the cap,
/// or the lint itself.
fn workflow_overrides(text: &str) -> Vec<String> {
    let text = decode_escapes(text).to_lowercase();
    ["unsafe_code", "unsafe-code", "cap-lints", "rustflags"]
        .into_iter()
        .filter(|word| text.contains(word))
        .map(str::to_string)
        .collect()
}

/// `line` without a trailing `# comment`, minding quoted strings.
fn strip_toml_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' => return &line[..i],
            None => {}
        }
    }
    line
}

/// `[ lints . "rust" ]` as `[lints.rust]`.
fn toml_header(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect()
}

/// A key without quotes and escapes, lower-cased.
fn toml_key(key: &str) -> String {
    decode_escapes(key.trim().trim_matches(['"', '\''])).to_lowercase()
}

/// A TOML string value (`"basic"` with escapes or `'literal'`).
fn toml_string(value: &str) -> Option<String> {
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return Some(decode_escapes(inner));
    }
    value
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .map(str::to_string)
}

/// A lint level, given plainly or as `{ level = "..", priority = .. }`.
fn toml_level(value: &str) -> Option<String> {
    let level = match value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
        Some(table) => table.split(',').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (toml_key(key) == "level").then(|| value.trim().to_string())
        })?,
        None => value.to_string(),
    };
    let level = toml_string(&level)?.to_lowercase();
    ["forbid", "deny", "warn", "allow"]
        .contains(&level.as_str())
        .then_some(level)
}

/// Text with the escapes TOML and YAML strings resolve (`\u002d`,
/// `\U0000002d`, `\x2d`) resolved and backslash line continuations joined,
/// so a flag is found however it is spelled.
fn decode_escapes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            let width = match chars.get(i + 1) {
                Some('u') => 4,
                Some('U') => 8,
                Some('x') => 2,
                _ => 0,
            };
            let hex: String = chars.iter().skip(i + 2).take(width).collect();
            if width > 0
                && let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
            {
                out.push(c);
                i += 2 + width;
                continue;
            }
            let mut j = i + 1;
            while chars
                .get(j)
                .is_some_and(|c| *c == ' ' || *c == '\t' || *c == '\r')
            {
                j += 1;
            }
            if chars.get(j) == Some(&'\n') {
                while chars.get(j).is_some_and(|c| c.is_whitespace()) {
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn has_module_header(source: &str) -> bool {
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("//!") {
            return true;
        }
        if !line.is_empty() {
            return false;
        }
    }
    false
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    /// `r#name`: never a keyword, but the same name for a lint.
    RawIdent(String),
    Punct(char),
    /// String-like literal with its contents (escapes left as written).
    Str(String),
    /// Numbers, character literals, lifetimes and labels.
    Other,
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    line: usize,
}

impl Token {
    fn is_punct(&self, c: char) -> bool {
        self.tok == Tok::Punct(c)
    }

    fn is_keyword(&self, word: &str) -> bool {
        matches!(&self.tok, Tok::Ident(name) if name == word)
    }

    /// The name for lint and attribute matching, raw or not.
    fn name(&self) -> Option<&str> {
        match &self.tok {
            Tok::Ident(name) | Tok::RawIdent(name) => Some(name),
            _ => None,
        }
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = source.chars().collect();
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    let mut tokens = Vec::new();
    let mut line = 1;
    let mut i = 0;
    // A first line `#!...` that does not open an inner attribute is a
    // shebang, which rustc drops.
    if source.starts_with("#!") && !source[2..].trim_start().starts_with('[') {
        while i < chars.len() && chars[i] != '\n' {
            i += 1;
        }
    }

    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            let start = line;
            let mut depth = 1;
            i += 2;
            while depth > 0 {
                match (at(i), at(i + 1)) {
                    ('\0', _) if i >= chars.len() => {
                        return Err(format!("block comment from line {start} never ends"));
                    }
                    ('/', '*') => {
                        depth += 1;
                        i += 2;
                    }
                    ('*', '/') => {
                        depth -= 1;
                        i += 2;
                    }
                    ('\n', _) => {
                        line += 1;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
        } else if let Some((end, text)) = raw_string(&chars, i) {
            let start = line;
            line += chars[i..end].iter().filter(|&&c| c == '\n').count();
            tokens.push(Token {
                tok: Tok::Str(text),
                line: start,
            });
            i = end;
        } else if c == '"' || ((c == 'b' || c == 'c') && at(i + 1) == '"') {
            let start = line;
            i += if c == '"' { 1 } else { 2 };
            let from = i;
            loop {
                match at(i) {
                    '\0' if i >= chars.len() => {
                        return Err(format!("string from line {start} never ends"));
                    }
                    '\\' => {
                        if at(i + 1) == '\n' {
                            line += 1;
                        }
                        i += 2;
                    }
                    '"' => break,
                    '\n' => {
                        line += 1;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
            tokens.push(Token {
                tok: Tok::Str(chars[from..i].iter().collect()),
                line: start,
            });
            i += 1;
        } else if c == '\'' || (c == 'b' && at(i + 1) == '\'') {
            if c == 'b' {
                i += 1;
            }
            if at(i + 1) == '\\' {
                // Escaped character: skip the escape, then up to the quote
                // ('\n', '\'', '\u{1F512}').
                i += 3;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if at(i + 2) == '\'' {
                i += 3;
            } else {
                // Lifetime or label.
                i += 1;
                while i < chars.len() && is_ident_continue(chars[i]) {
                    i += 1;
                }
            }
            tokens.push(Token {
                tok: Tok::Other,
                line,
            });
        } else if c == 'r' && at(i + 1) == '#' && is_ident_start(at(i + 2)) {
            let from = i + 2;
            i = from;
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }
            tokens.push(Token {
                tok: Tok::RawIdent(chars[from..i].iter().collect()),
                line,
            });
        } else if is_ident_start(c) {
            let from = i;
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }
            tokens.push(Token {
                tok: Tok::Ident(chars[from..i].iter().collect()),
                line,
            });
        } else if c.is_ascii_digit() {
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }
            tokens.push(Token {
                tok: Tok::Other,
                line,
            });
        } else {
            tokens.push(Token {
                tok: Tok::Punct(c),
                line,
            });
            i += 1;
        }
    }
    Ok(tokens)
}

/// A raw string (`r"…"`, `r#"…"#`, `br"…"`, `cr#"…"#`) starting at `i`: the
/// index after it and its contents.
fn raw_string(chars: &[char], i: usize) -> Option<(usize, String)> {
    let mut j = i;
    if matches!(chars.get(j), Some('b' | 'c')) {
        j += 1;
    }
    if chars.get(j) != Some(&'r') {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if chars.get(j) != Some(&'"') {
        return None;
    }
    let from = j + 1;
    let mut k = from;
    while k < chars.len() {
        if chars[k] == '"' && (1..=hashes).all(|n| chars.get(k + n) == Some(&'#')) {
            return Some((k + 1 + hashes, chars[from..k].iter().collect()));
        }
        k += 1;
    }
    None
}

/// Tokens plus the delimiter structure: the partner of every bracket and the
/// innermost `{` around every token.
struct Parsed {
    tokens: Vec<Token>,
    partner: Vec<Option<usize>>,
    enclosing_brace: Vec<Option<usize>>,
    /// Whether the token sits in the arguments of a macro call (`name!(..)`).
    in_macro_call: Vec<bool>,
}

/// Keywords that can stand before a `!` that negates rather than calls.
const NOT_MACRO_NAMES: &[&str] = &[
    "if", "while", "return", "match", "in", "else", "break", "let", "mut", "yield",
];

fn parse(source: &str) -> Result<Parsed, String> {
    let tokens = tokenize(source)?;
    let mut partner = vec![None; tokens.len()];
    let mut enclosing_brace = vec![None; tokens.len()];
    let mut in_macro_call = vec![false; tokens.len()];
    let mut stack: Vec<usize> = Vec::new();
    let is_macro_call = |open: usize| {
        open >= 2
            && tokens[open - 1].is_punct('!')
            && matches!(&tokens[open - 2].tok, Tok::Ident(name) | Tok::RawIdent(name)
                if !NOT_MACRO_NAMES.contains(&name.as_str()))
    };
    for (i, token) in tokens.iter().enumerate() {
        enclosing_brace[i] = stack
            .iter()
            .rev()
            .find(|&&open| tokens[open].is_punct('{'))
            .copied();
        in_macro_call[i] = stack.iter().any(|&open| is_macro_call(open));
        let Tok::Punct(c) = token.tok else {
            continue;
        };
        match c {
            '(' | '[' | '{' => stack.push(i),
            ')' | ']' | '}' => {
                let want = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                match stack.pop() {
                    Some(open) if tokens[open].is_punct(want) => {
                        partner[open] = Some(i);
                        partner[i] = Some(open);
                    }
                    _ => return Err(format!("unbalanced `{c}` on line {}", token.line)),
                }
            }
            _ => {}
        }
    }
    if let Some(&open) = stack.last() {
        return Err(format!(
            "unclosed delimiter from line {}",
            tokens[open].line
        ));
    }
    Ok(Parsed {
        tokens,
        partner,
        enclosing_brace,
        in_macro_call,
    })
}

struct Attribute {
    hash: usize,
    open: usize,
    close: usize,
    inner: bool,
}

fn attributes(p: &Parsed) -> Vec<Attribute> {
    let t = &p.tokens;
    let mut out = Vec::new();
    for (i, token) in t.iter().enumerate() {
        if !token.is_punct('#') {
            continue;
        }
        let (inner, open) = if t.get(i + 1).is_some_and(|t| t.is_punct('!'))
            && t.get(i + 2).is_some_and(|t| t.is_punct('['))
        {
            (true, i + 2)
        } else if t.get(i + 1).is_some_and(|t| t.is_punct('[')) {
            (false, i + 1)
        } else {
            continue;
        };
        out.push(Attribute {
            hash: i,
            open,
            close: p.partner[open].expect("balanced attribute"),
            inner,
        });
    }
    out
}

struct Exception {
    anchor: String,
    line: usize,
    start: usize,
    end: usize,
}

struct Report {
    exceptions: Vec<Exception>,
    unsafe_sites: Vec<usize>,
    problems: Vec<(usize, String)>,
}

const RELAXING_LEVELS: &[&str] = &["allow", "expect", "warn"];
const STRICT_LEVELS: &[&str] = &["deny", "forbid"];
const UNSAFE_ATTRIBUTES: &[&str] = &[
    "no_mangle",
    "export_name",
    "link_section",
    "allow_internal_unsafe",
];

fn scan(source: &str) -> Result<Report, String> {
    let p = parse(source)?;
    let t = &p.tokens;
    let attrs = attributes(&p);
    let inside_attribute = |i: usize| attrs.iter().find(|a| a.open < i && i < a.close);
    let mut problems = Vec::new();
    let mut exceptions: Vec<Exception> = Vec::new();
    let mut relaxing: Vec<usize> = Vec::new();

    // Every mention of the lint must sit in a lint attribute. A relaxing one
    // is an opt-in; anything else (say, a macro argument that a macro turns
    // into an attribute) is a finding, because the scan cannot see where it
    // ends up.
    for (i, token) in t.iter().enumerate() {
        if !token
            .name()
            .is_some_and(|name| name.eq_ignore_ascii_case("unsafe_code"))
        {
            continue;
        }
        let Some(attr) = inside_attribute(i) else {
            problems.push((
                token.line,
                "`unsafe_code` outside an attribute (a macro could turn it into an \
                 allow)"
                    .to_string(),
            ));
            continue;
        };
        let level = innermost_call(&p, attr, i);
        match level {
            Some(level) if RELAXING_LEVELS.contains(&level) => {
                if !relaxing.contains(&attr.hash) {
                    relaxing.push(attr.hash);
                }
            }
            Some(level) if STRICT_LEVELS.contains(&level) => {}
            _ => problems.push((
                token.line,
                "`unsafe_code` in an attribute that is not a lint level".to_string(),
            )),
        }
    }
    for attr in attrs.iter().filter(|a| relaxing.contains(&a.hash)) {
        exceptions.push(exception(&p, &attrs, attr));
    }

    let mut unsafe_sites = Vec::new();
    for (i, token) in t.iter().enumerate() {
        // `global_asm` counts under any spelling and wherever it is named:
        // `r#global_asm!`, or imported under another name.
        let site = token.is_keyword("unsafe")
            || token.name() == Some("global_asm")
            || (token
                .name()
                .is_some_and(|name| UNSAFE_ATTRIBUTES.contains(&name))
                && inside_attribute(i).is_some());
        if site {
            unsafe_sites.push(i);
            if !exceptions.iter().any(|e| e.start <= i && i <= e.end) {
                problems.push((
                    token.line,
                    format!(
                        "`{}` outside every item that allows unsafe_code",
                        token.name().unwrap_or("unsafe")
                    ),
                ));
            }
        }
        // The `include!` macro under any spelling: called (`include!`,
        // `r#include!`), named by path (`std::include`), renamed on import
        // (`include as inc`) or handed to another macro (`wrap!(include)`).
        // A field or variable of that name is none of these, and neither is
        // rust-embed's `#[include = ..]`.
        let after_path = i >= 2 && t[i - 1].is_punct(':') && t[i - 2].is_punct(':');
        let called_or_renamed = t
            .get(i + 1)
            .is_some_and(|n| n.is_punct('!') || n.is_keyword("as"));
        if token.name() == Some("include")
            && inside_attribute(i).is_none()
            && (called_or_renamed || after_path || p.in_macro_call[i])
        {
            problems.push((
                token.line,
                "include! brings in code this scan does not read".to_string(),
            ));
        }
    }

    for attr in &attrs {
        let body = &t[attr.open + 1..attr.close];
        if let [name, eq, value] = body
            && name.is_keyword("path")
            && eq.is_punct('=')
            && let Tok::Str(target) = &value.tok
            && !stays_in_tree(target)
        {
            problems.push((
                name.line,
                format!("#[path = \"{target}\"] is not a .rs file the scan reads"),
            ));
        }
    }

    Ok(Report {
        exceptions,
        unsafe_sites,
        problems,
    })
}

/// The name in front of the innermost parenthesis around token `i` inside
/// `attr`: `allow` for `#[cfg_attr(x, allow(unsafe_code))]`.
fn innermost_call<'a>(p: &'a Parsed, attr: &Attribute, i: usize) -> Option<&'a str> {
    (attr.open + 1..i)
        .rfind(|&j| p.tokens[j].is_punct('(') && p.partner[j].is_some_and(|close| close > i))
        .and_then(|j| p.tokens.get(j - 1))
        .and_then(Token::name)
}

/// What a relaxing attribute covers, and the name of that item.
fn exception(p: &Parsed, attrs: &[Attribute], attr: &Attribute) -> Exception {
    let t = &p.tokens;
    let line = t[attr.hash].line;
    if attr.inner {
        return match p.enclosing_brace[attr.hash] {
            None => Exception {
                anchor: "file".to_string(),
                line,
                start: 0,
                end: t.len(),
            },
            Some(brace) => Exception {
                anchor: anchor(&t[header_start(p, brace)..brace]),
                line,
                start: brace,
                end: p.partner[brace].expect("balanced block"),
            },
        };
    }

    // An outer attribute covers the item after it: past any further
    // attributes, up to its `;` or to the end of its `{ … }` body.
    let mut item = attr.close + 1;
    while let Some(next) = attrs.iter().find(|a| a.hash == item && !a.inner) {
        item = next.close + 1;
    }
    let mut j = item;
    let end = loop {
        let Some(token) = t.get(j) else {
            break t.len();
        };
        match token.tok {
            Tok::Punct('(' | '[') => j = p.partner[j].expect("balanced") + 1,
            Tok::Punct('{') => break p.partner[j].expect("balanced"),
            Tok::Punct(';') => break j,
            Tok::Punct(')' | ']' | '}') => break j.saturating_sub(1),
            _ => j += 1,
        }
    };
    Exception {
        anchor: anchor(&t[item..end.min(t.len()).max(item)]),
        line,
        start: attr.hash,
        end,
    }
}

/// Start of the header that opens the block at `brace` (`#[cfg(..)] mod x`).
fn header_start(p: &Parsed, brace: usize) -> usize {
    let mut j = brace;
    while j > 0 {
        let prev = j - 1;
        match p.tokens[prev].tok {
            Tok::Punct(';' | '{' | '}') => return j,
            Tok::Punct(')' | ']') => j = p.partner[prev].expect("balanced"),
            _ => j = prev,
        }
    }
    0
}

const ITEM_KEYWORDS: &[&str] = &[
    "fn",
    "mod",
    "struct",
    "enum",
    "trait",
    "union",
    "type",
    "static",
    "const",
    "impl",
    "macro_rules",
    "let",
    "use",
    "extern",
];
const QUALIFIERS: &[&str] = &["fn", "unsafe", "safe", "async", "extern", "const", "crate"];

/// `fn name`, `mod name`, `impl Trait`, ...: the item keyword and the name
/// after it, ignoring attributes, visibility and qualifiers.
fn anchor(header: &[Token]) -> String {
    let mut depth = 0usize;
    let mut first = None;
    for (i, token) in header.iter().enumerate() {
        match token.tok {
            Tok::Punct('(' | '[') => depth += 1,
            Tok::Punct(')' | ']') => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth > 0 {
            continue;
        }
        let Some(keyword) = token.name().filter(|name| ITEM_KEYWORDS.contains(name)) else {
            continue;
        };
        let after =
            if keyword == "macro_rules" && header.get(i + 1).is_some_and(|t| t.is_punct('!')) {
                header.get(i + 2)
            } else {
                header.get(i + 1)
            };
        if let Some(name) = after
            .and_then(Token::name)
            .filter(|name| !QUALIFIERS.contains(name))
        {
            return format!("{keyword} {name}");
        }
        if first.is_none() {
            first = Some(keyword);
        }
    }
    first.unwrap_or("item").to_string()
}

// ------------------------------------------------------ scanner self-test --

fn unsafe_lines(source: &str) -> Vec<usize> {
    let report = scan(source).expect("scan");
    let tokens = tokenize(source).expect("tokenize");
    let mut lines: Vec<usize> = report
        .unsafe_sites
        .iter()
        .map(|&i| tokens[i].line)
        .collect();
    lines.dedup();
    lines
}

fn anchors(source: &str) -> Vec<String> {
    scan(source)
        .expect("scan")
        .exceptions
        .into_iter()
        .map(|e| e.anchor)
        .collect()
}

fn problem_lines(source: &str) -> Vec<usize> {
    let mut lines: Vec<usize> = scan(source)
        .expect("scan")
        .problems
        .into_iter()
        .map(|(line, _)| line)
        .collect();
    lines.sort_unstable();
    lines.dedup();
    lines
}

#[test]
fn comments_strings_and_characters_are_not_code() {
    let source = r##"
        // unsafe { a() }
        /// unsafe in a doc comment, #[allow(unsafe_code)]
        /* unsafe /* nested unsafe */ still a comment: unsafe */
        const A: &str = "unsafe { b() } #[allow(unsafe_code)]";
        const B: &str = r#"unsafe "quoted" unsafe"#;
        const C: &[u8] = b"unsafe";
        const D: &[u8] = br"unsafe";
        const E: char = '"';
        const F: char = '\'';
        const G: char = '\u{1F512}';
        fn h<'a>(x: &'a str) -> &'a str { 'outer: loop { break 'outer x; } }
        fn r#unsafe_named() {}
        const I: &str = "escaped \" unsafe";
    "##;
    assert_eq!(unsafe_lines(source), Vec::<usize>::new());
    assert_eq!(problem_lines(source), Vec::<usize>::new());
    assert!(anchors(source).is_empty());
}

#[test]
fn every_unsafe_form_is_found() {
    let source = r#"
        unsafe impl Send for X {}
        unsafe extern "C" { safe fn f(); }
        unsafe fn g() {}
        macro_rules! m { () => { unsafe { h() } } }
        #[unsafe(no_mangle)] fn i() {}
        #[no_mangle] fn j() {}
        #[cfg_attr(x, export_name = "k")] fn k() {}
        #[link_section = ".x"] static L: u8 = 0;
        core::arch::global_asm!("");
        trait T { unsafe fn n(&self); }
        fn o() { let _ = unsafe { p() }; }
        core::arch::r#global_asm!("");
        use core::arch::global_asm as ga;
        #[allow_internal_unsafe] macro_rules! q { () => {} }
    "#;
    let all: Vec<usize> = (2..=15).collect();
    assert_eq!(unsafe_lines(source), all);
    assert_eq!(problem_lines(source), all);
}

#[test]
fn relaxing_attributes_are_found_and_strict_ones_are_not() {
    let source = r#"
        #[allow(unsafe_code)] fn a() {}
        #[expect(unsafe_code)] fn b() {}
        #[warn(unsafe_code)] fn c() {}
        #[allow(dead_code, unsafe_code)] fn d() {}
        #[cfg_attr(test, allow(unsafe_code))] fn e() {}
        #[allow(r#unsafe_code)] fn f() {}
        # [ allow ( unsafe_code ) ] fn g() {}
        #[deny(unsafe_code)] fn h() {}
        #[cfg_attr(test, forbid(unsafe_code))] fn i() {}
        mod m { #![allow(unsafe_code)] }
    "#;
    assert_eq!(
        anchors(source),
        vec![
            "fn a", "fn b", "fn c", "fn d", "fn e", "fn f", "fn g", "mod m"
        ]
    );
    assert_eq!(problem_lines(source), Vec::<usize>::new());
}

#[test]
fn the_lint_name_outside_an_attribute_is_a_finding() {
    // A macro can build the attribute from its argument, out of the scan's
    // sight, so the name alone is enough.
    let source = r#"
        macro_rules! relax { ($l:ident) => { #[allow($l)] fn x() { unsafe {} } } }
        relax!(unsafe_code);
    "#;
    assert_eq!(problem_lines(source), vec![2, 3]);
}

#[test]
fn an_allow_covers_its_item_and_nothing_after_it() {
    let source = r#"
        #[cfg(unix)]
        #[allow(unsafe_code)]
        #[inline]
        pub(crate) fn covered() -> [u8; 1] { unsafe { a() } }
        fn not_covered() { unsafe { b() } }
        mod block {
            #![allow(unsafe_code)]
            fn inside() { unsafe { c() } }
        }
        fn after_block() { unsafe { d() } }
        #[allow(unsafe_code)]
        unsafe impl Send for Covered {}
        #[allow(unsafe_code)]
        const fn qualified() {}
    "#;
    assert_eq!(
        anchors(source),
        vec!["fn covered", "mod block", "impl Send", "fn qualified"]
    );
    assert_eq!(problem_lines(source), vec![6, 11]);
}

#[test]
fn a_file_level_allow_covers_the_whole_file() {
    let source = "//! Why.\n\n#![allow(unsafe_code)]\n\nfn a() { unsafe { b() } }\n";
    assert_eq!(anchors(source), vec!["file"]);
    assert_eq!(problem_lines(source), Vec::<usize>::new());
    assert!(has_module_header(source));
    assert!(!has_module_header(
        "#![allow(unsafe_code)]\n//! Too late.\n"
    ));
}

#[test]
fn code_the_scan_cannot_read_is_a_finding() {
    let source = r#"
        include!("elsewhere.rs");
        #[path = "../outside.rs"] mod outside;
        #[path = "/abs/outside.rs"] mod absolute;
        #[path = "clipboard_macos.rs"] mod inside;
        const S: &str = include_str!("data.txt");
        r#include!("hidden.inc");
        use std::include as inc;
        wrap!(include);
        #[path = "hidden.inc"] mod hidden;
        #[include = "icons/**/*.svg"] struct Assets;
        struct Filter { include: bool }
        fn f(x: Filter) -> bool { let include = x.include; include }
    "#;
    assert_eq!(problem_lines(source), vec![2, 3, 4, 7, 8, 9, 10]);
}

#[test]
fn a_shebang_line_is_not_code() {
    let source = "#!/usr/bin/env run-cargo-script --unsafe\nfn a() {}\n";
    assert_eq!(problem_lines(source), Vec::<usize>::new());
    // An inner attribute is not a shebang.
    assert_eq!(anchors("#![allow(unsafe_code)]\n"), vec!["file"]);
}

#[test]
fn source_discovery_skips_only_target_and_git() {
    let dir = tempfile::tempdir().expect("temp dir");
    for file in [
        "src/a.rs",
        "tools/b.rs",
        "nested/target/c.rs",
        "target/d.rs",
        ".git/e.rs",
        "src/notes.txt",
    ] {
        let path = dir.path().join(file);
        fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        fs::write(&path, "").expect("write");
    }
    let found: Vec<String> = rust_sources(dir.path())
        .into_iter()
        .map(|(rel, _)| rel)
        .collect();
    assert_eq!(found, vec!["nested/target/c.rs", "src/a.rs", "tools/b.rs"]);
}

#[test]
fn build_targets_outside_the_tree_are_found() {
    let manifest = r#"
[package]
build = "build.rs"
[lib]
path = "src/lib.rs"
[[bin]]
path = "tools/helper.inc"
[[bin]]
path = "../outside.rs" # a comment
[dependencies.local]
path = "../local"
[patch.crates-io.other]
path = "../other"
[package.metadata]
build = false
"#;
    assert_eq!(
        stray_target_paths(manifest),
        vec![
            "Cargo.toml:7: path = \"tools/helper.inc\"",
            "Cargo.toml:9: path = \"../outside.rs\""
        ]
    );
}

#[test]
fn lint_flags_outside_cargo_toml_are_found() {
    assert_eq!(
        cargo_config_overrides("[build]\nrustflags = [\"--cap\\u002dlints=allow\"]\n"),
        vec!["cap-lints"]
    );
    assert_eq!(
        cargo_config_overrides("[alias]\nt = \"test --config x.toml\"\ninclude = [\"a.toml\"]\n"),
        vec!["--config", "include"]
    );
    assert_eq!(
        cargo_config_overrides("[target.x]\nrustflags = [\"-A\", \"UNSAFE\\x5fCODE\"]\n"),
        vec!["unsafe_code"]
    );
    assert!(
        cargo_config_overrides("[target.x]\nrustflags = [\"--cfg\", \"aes_armv8\"]\n").is_empty()
    );
    assert_eq!(
        workflow_overrides("env:\n  CARGO_BUILD_RUSTFLAGS: \"-A unsafe\\\n    -code\"\n"),
        vec!["unsafe-code", "rustflags"]
    );
    assert!(workflow_overrides("run: git-cliff --config cliff.toml\n").is_empty());
}

#[test]
fn malformed_source_is_an_error_not_a_pass() {
    assert!(scan("fn a() { \"open string }").is_err());
    assert!(scan("fn a() { /* open comment }").is_err());
    assert!(scan("fn a() { (] }").is_err());
}

#[test]
fn lint_levels_are_read_from_the_lints_table() {
    let level = unsafe_code_level;
    assert_eq!(
        level("[package]\nunsafe_code = \"allow\"\n[lints.rust]\nunsafe_code = \"deny\"\n"),
        Ok("deny".to_string())
    );
    assert_eq!(
        level("[lints.rust]\nunsafe_code = \"allow\" # was \"deny\"\n"),
        Ok("allow".to_string())
    );
    assert_eq!(
        level("[ lints . \"rust\" ]\nunsafe_code = { level = \"warn\", priority = 1 }\n"),
        Ok("warn".to_string())
    );
    assert_eq!(
        level("[lints.rust]\n\"unsafe_code\" = 'forbid'\n"),
        Ok("forbid".to_string())
    );
    assert!(level("[lints.rust]\nunsafe_code = \"deny\"\nunsafe-code = \"allow\"\n").is_err());
    assert!(level("[lints]\nrust.unsafe_code = \"allow\"\n").is_err());
    assert!(level("[lints.rust]\nunsafe_code = \"loud\"\n").is_err());
    assert!(level("[lints.rust]\n").is_err());
}
