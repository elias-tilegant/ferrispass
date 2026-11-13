//! Architecture rules checked by reading the source.
//!
//! New `unsafe` goes only into `src/platform/`, one opt-in per file,
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
const PLATFORM_MODULES: &[&str] = &[
    "src/platform/secret_fd.rs",
    "src/platform/secure_input.rs",
    "src/platform/system_idle.rs",
];

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

// ---------------------------------------------- clipboard content reads --

/// `NSPasteboard` and `NSPasteboardItem` methods that return what is on the
/// clipboard. Reading another app's contents is what shows the "paste from
/// other apps" prompt. The clipboard code writes, clears and asks for types,
/// and names these only in its tests. A name matches with any suffix, so
/// `readObjectsForClasses_options` counts.
const PASTEBOARD_CONTENT_READERS: &[&str] = &[
    "stringForType",
    "dataForType",
    "propertyListForType",
    "pasteboardItems",
    "readObjectsForClasses",
];

/// The clipboard service, the pasteboard surface it works through, and both
/// backends.
const CLIPBOARD_SOURCES: &[&str] = &[
    "src/app/services/clipboard.rs",
    "src/clipboard.rs",
    "src/clipboard_macos.rs",
    "src/clipboard_portable.rs",
];

#[test]
fn clipboard_code_never_reads_pasteboard_content() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    for rel in CLIPBOARD_SOURCES {
        let source = fs::read_to_string(root.join(rel))
            .unwrap_or_else(|err| panic!("cannot read {rel}: {err}"));
        let reads = content_reads_outside_tests(&source)
            .unwrap_or_else(|err| panic!("cannot tokenize {rel}: {err}"));
        for (line, name) in reads {
            found.push(format!("{rel}:{line}: {name}"));
        }
    }
    assert!(
        found.is_empty(),
        "the clipboard code reads pasteboard contents outside its tests:\n{}",
        found.join("\n")
    );
}

/// Every name from [`PASTEBOARD_CONTENT_READERS`] in code, with its line,
/// except inside items switched off by `#[cfg(test)]`.
fn content_reads_outside_tests(source: &str) -> Result<Vec<(usize, String)>, String> {
    let p = parse(source)?;
    let test_items = cfg_test_items(&p);
    Ok(p.tokens
        .iter()
        .enumerate()
        .filter(|(i, _)| !test_items.iter().any(|item| item.contains(i)))
        .filter_map(|(_, token)| {
            let name = token.name()?;
            PASTEBOARD_CONTENT_READERS
                .iter()
                .any(|reader| name.starts_with(reader))
                .then(|| (token.line, name.to_string()))
        })
        .collect())
}

/// Token ranges of what an outer `#[cfg(test)]` switches off. Only that exact
/// attribute counts; `cfg(any(test, ..))` and the like are scanned, which errs
/// on the side of a finding.
fn cfg_test_items(p: &Parsed) -> Vec<std::ops::RangeInclusive<usize>> {
    let t = &p.tokens;
    attributes(p)
        .into_iter()
        .filter(|attr| {
            let inside = &t[attr.open + 1..attr.close];
            !attr.inner
                && inside.len() == 4
                && inside[0].is_keyword("cfg")
                && inside[1].is_punct('(')
                && inside[2].is_keyword("test")
                && inside[3].is_punct(')')
        })
        .map(|attr| attr.hash..=item_end(p, attr.close + 1))
        .collect()
}

/// The last token of the item, statement, field or arm that starts at
/// `start`: the `}` closing its first brace block, or the `;` or `,` that ends
/// it. Parentheses and brackets on the way are skipped whole, and a closing
/// delimiter means the enclosing block ended first.
fn item_end(p: &Parsed, start: usize) -> usize {
    let t = &p.tokens;
    let mut i = start;
    while i < t.len() {
        match t[i].tok {
            Tok::Punct('{') => return p.partner[i].expect("balanced brace"),
            Tok::Punct(';' | ',') => return i,
            Tok::Punct('(' | '[') => i = p.partner[i].expect("balanced delimiter"),
            Tok::Punct('}' | ')' | ']') => return i.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    t.len().saturating_sub(1)
}

#[test]
fn content_reads_are_found_outside_tests_only() {
    let source = r##"
        // p.stringForType(&t) in a comment
        fn real(p: &NSPasteboard) { let _ = p.stringForType(&t); }
        const NOTE: &str = "dataForType( in a string";
        #[cfg(test)]
        mod tests { fn helper(p: &NSPasteboard) { p.dataForType(&t); } }
        #[cfg(test)]
        #[allow(dead_code)]
        fn another(p: &NSPasteboard) -> Vec<[u8; 2]> { p.pasteboardItems(); vec![] }
        struct S { #[cfg(test)] field: u8, other: Kind }
        fn after(p: &NSPasteboard) { p.readObjectsForClasses_options(&a, None); }
        #[cfg(not(test))]
        fn live(p: &NSPasteboard) { p.propertyListForType(&t); }
    "##;
    assert_eq!(
        content_reads_outside_tests(source).expect("scan"),
        vec![
            (3, "stringForType".to_string()),
            (11, "readObjectsForClasses_options".to_string()),
            (13, "propertyListForType".to_string()),
        ]
    );
}

// ------------------------------------------------------------- idle clock --

/// Files that may call `activity::touch`, with the source each call names.
/// The idle clock behind the auto-lock counts only input the user gives
/// FerrisPass (see `src/app/services/activity.rs`), so every new caller is a
/// line here and stands out in review.
const ACTIVITY_CALL_SITES: &[(&str, &str)] = &[
    ("src/ui/legacy/shell/render.rs", "WindowInput"),
    // A press of the quick access shortcut, when the one loop for the global
    // shortcuts hands it on.
    ("src/app/services/hotkeys.rs", "Hotkey"),
    // Keys and clicks in the quick access panel.
    ("src/ui/windows/panel/view.rs", "Panel"),
    // The test of the clock as the app builds it. Its second thread stands in
    // for the one that will listen for the global shortcuts; it is not input.
    ("tests/idle_clock.rs", "Hotkey"),
];

/// Every call of `activity::reset`, one line per call, with the function it
/// is in. A reset sets the clock just as input does, so one in background
/// work could keep the vault from ever locking. A second call in a listed
/// function needs a second line.
const RESET_CALL_SITES: &[(&str, &str)] = &[
    // The auto-lock task counts from the moment it starts, so a vault opened
    // after a long pause is not locked on its first check.
    ("src/ui/legacy/shell/tasks.rs", "sync_auto_lock_task"),
    // The test of the clock as the app builds it sets a time to read back...
    (
        "tests/idle_clock.rs",
        "the_app_has_one_idle_clock_for_every_thread",
    ),
    // ...and sets it again from a second thread, which the first has to see.
    (
        "tests/idle_clock.rs",
        "the_app_has_one_idle_clock_for_every_thread",
    ),
];

// ------------------------------------------------- the locked quick access panel --

/// Every call that can raise the Touch ID prompt or lead to it, one line per
/// call, with the function it is in and the name it calls:
/// `begin_biometric_unlock` starts an attempt in the app, `retrieve` shows the
/// prompt and reads the stored password, `unlock_from_lock_screen` and
/// `unlock_from_panel` are the two ways into the shared flow, and `confirm`
/// turns something that happened in the quick access panel into the token the
/// panel's way in takes. A token only says that `confirm` was asked with
/// Return or the unlock button, so the function that asks has to be the one
/// that handles that key or click; a timer or an activation handler on this
/// list would stand out in review, as would any other new caller.
const TOUCH_ID_CALL_SITES: &[(&str, &str, &str)] = &[
    // The one flow for the lock screen and the quick access panel: the
    // attempt, then the prompt in the background.
    (
        "src/app/unlock/touch_id.rs",
        "run",
        "begin_biometric_unlock",
    ),
    ("src/app/unlock/touch_id.rs", "run", "retrieve"),
    // The unlock screen: the Touch ID button and its automatic prompt.
    (
        "src/ui/legacy/shell/unlock.rs",
        "start_biometric_unlock",
        "unlock_from_lock_screen",
    ),
    // The command line, with --touch-id, or at a terminal for an enrolled
    // vault.
    ("src/cli.rs", "unlock_password", "retrieve"),
    // The locked quick access panel: Return, and a click on its unlock
    // button. Each asks for a token in its own handler...
    ("src/ui/windows/panel/locked.rs", "on_return", "confirm"),
    (
        "src/ui/windows/panel/locked.rs",
        "on_unlock_click",
        "confirm",
    ),
    // ...and hands it to the panel's way into the flow.
    (
        "src/ui/windows/panel/locked.rs",
        "start_unlock",
        "unlock_from_panel",
    ),
];

/// The names whose calls [`TOUCH_ID_CALL_SITES`] lists.
const TOUCH_ID_ENTRIES: &[&str] = &[
    "begin_biometric_unlock",
    "retrieve",
    "unlock_from_lock_screen",
    "unlock_from_panel",
    "confirm",
];

#[test]
fn touch_id_starts_only_where_listed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sources: Vec<(String, String)> = rust_sources(root)
        .into_iter()
        .map(|(rel, path)| {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
            (rel, source)
        })
        .collect();
    let test_only = test_only_files(&sources).unwrap_or_else(|err| panic!("cannot tokenize {err}"));
    let mut unused: Vec<(&str, &str, &str)> = TOUCH_ID_CALL_SITES.to_vec();
    let mut problems = Vec::new();
    for (rel, source) in &sources {
        if test_only.contains(rel) {
            continue;
        }
        let calls =
            touch_id_calls(source).unwrap_or_else(|err| panic!("cannot tokenize {rel}: {err}"));
        for (line, call) in calls {
            match call {
                Ok((function, name)) => match unused
                    .iter()
                    .position(|&(f, func, called)| f == rel && func == function && called == name)
                {
                    Some(at) => {
                        unused.remove(at);
                    }
                    None => problems.push(format!(
                        "{rel}:{line}: calls `{name}` in `fn {function}`, and \
                         TOUCH_ID_CALL_SITES has no line left for it. Each call can raise the \
                         Touch ID prompt; if this one should, add a line with its reason."
                    )),
                },
                Err(problem) => problems.push(format!("{rel}:{line}: {problem}")),
            }
        }
    }
    for (file, function, name) in unused {
        problems.push(format!(
            "{file}: listed as calling `{name}` in `fn {function}`, but no such call is left. \
             Remove the line."
        ));
    }
    assert!(
        problems.is_empty(),
        "Touch ID can start outside the listed places:\n{}",
        problems.join("\n")
    );
}

/// Each use of a name from [`TOUCH_ID_ENTRIES`] in code, outside items
/// switched off by `#[cfg(test)]`: `Ok` with the function a call is in and
/// the name it calls, or a problem. A definition (`fn retrieve`) is not a
/// call. The name used any other way, imported or passed as a value, is a
/// problem, because a call could hide behind it.
fn touch_id_calls(source: &str) -> Result<Vec<FoundEntryCall>, String> {
    let p = parse(source)?;
    let t = &p.tokens;
    let test_items = cfg_test_items(&p);
    let mut found = Vec::new();
    for i in 0..t.len() {
        let Some(name) = t[i].name().filter(|name| TOUCH_ID_ENTRIES.contains(name)) else {
            continue;
        };
        if test_items.iter().any(|item| item.contains(&i)) {
            continue;
        }
        if i > 0 && t[i - 1].is_keyword("fn") {
            continue;
        }
        let line = t[i].line;
        if !t.get(i + 1).is_some_and(|tok| tok.is_punct('(')) {
            found.push((
                line,
                Err(format!(
                    "`{name}` is named without being called, so a call could go unseen; \
                     call it directly"
                )),
            ));
            continue;
        }
        let call = enclosing_fn(&p, i)
            .map(|function| (function, name.to_string()))
            .ok_or_else(|| format!("calls `{name}` outside a function"));
        found.push((line, call));
    }
    Ok(found)
}

/// A line, and the function and name of the call found there, or what is
/// wrong with it.
type FoundEntryCall = (usize, Result<(String, String), String>);

#[test]
fn touch_id_calls_are_found_with_their_function() {
    let source = r##"
        // store.retrieve(&id) in a comment
        fn unlock(store: &dyn BiometricStore) { let _ = store.retrieve(&id, "p", options); }
        trait BiometricStore { fn retrieve(&self, id: &EnrollmentId) -> String; }
        fn start(state: &mut AppState) { spawn(async move { state.begin_biometric_unlock(cx); }); }
        fn full(store: &Store) { BiometricStore::retrieve(store, &id, "p", options); }
        const NOTE: &str = "retrieve(";
        fn later(store: &Store) { let f = BiometricStore::retrieve; }
        use crate::app::AppState::begin_biometric_unlock;
        fn on_timer(cx: &mut App) { let token = touch_id::confirm(PanelTrigger::ReturnKey); }
        fn on_open(state: Entity<AppState>, cx: &mut App) { touch_id::unlock_from_lock_screen(state, true, done, cx); }
        pub fn confirm(trigger: PanelTrigger) -> Option<Confirmed> { None }
        use crate::app::unlock::touch_id::unlock_from_panel;
        #[cfg(test)]
        mod tests { fn helper(store: &Store) { store.retrieve(&id, "p", o); } }
    "##;
    let found = touch_id_calls(source).expect("scan");
    let calls: Vec<(usize, String, String)> = found
        .iter()
        .filter_map(|(line, call)| {
            call.as_ref()
                .ok()
                .map(|(function, name)| (*line, function.clone(), name.clone()))
        })
        .collect();
    let problems: Vec<usize> = found
        .iter()
        .filter(|(_, call)| call.is_err())
        .map(|(line, _)| *line)
        .collect();
    let call =
        |line: usize, function: &str, name: &str| (line, function.to_string(), name.to_string());
    assert_eq!(
        calls,
        vec![
            call(3, "unlock", "retrieve"),
            call(5, "start", "begin_biometric_unlock"),
            call(6, "full", "retrieve"),
            call(10, "on_timer", "confirm"),
            call(11, "on_open", "unlock_from_lock_screen"),
        ]
    );
    assert_eq!(problems, vec![8, 9, 13]);
}

/// The file that holds what is typed into the quick access panel while it is
/// locked.
const TYPED_AHEAD_SOURCE: &str = "src/app/unlock/typed_ahead.rs";

/// Names through which typed text could leave the buffer: the file system,
/// the log, the print macros, formatting other than the hand-written `Debug`
/// that shows only a count, and derives (a derived `Debug` prints every
/// field).
const TEXT_OUTLETS: &[&str] = &[
    "fs",
    "File",
    "OpenOptions",
    "log",
    "tracing",
    "print",
    "println",
    "eprint",
    "eprintln",
    "dbg",
    "Display",
    "Serialize",
    "derive",
    "write_str",
    "write_fmt",
    "debug_struct",
    "debug_tuple",
];

/// The functions in the typed-ahead file that may touch the field `text`.
/// Each keeps the text in the buffer, counts it, wipes it, or hands it to
/// the caller as a `Zeroizing<String>`. Anything new that reads the text is
/// a line here and stands out in review.
const TYPED_TEXT_USERS: &[&str] = &[
    "begin",
    "push",
    "backspace",
    "len",
    "is_empty",
    "wipe_if_password_like",
    "wipe",
    "take_query",
];

/// Every method called on the field `text`, with the function it is called
/// in. Only `zeroize` overwrites the typed bytes. A method that shortens the
/// string (`clear`, `truncate`, `drain`) leaves them in its allocation until
/// the buffer drops, and one that regrows or shrinks it (`reserve`,
/// `push_str`, `shrink_to_fit`) frees the old allocation without
/// overwriting it. No safe test can read those bytes, so this list holds the
/// code to its shape instead: `wipe` overwrites, and no method beyond these
/// is called on the text. `String::push` is one of them, and it too would
/// regrow a string that is out of room. That `push` copies the text before
/// it gets there is held by the assertion in front of that call and the
/// tests in `typed_ahead.rs` that type up to the last free byte, not by
/// this list.
const TYPED_TEXT_CALLS: &[(&str, &str)] = &[
    // Copies the text into a bigger buffer when `c` doesn't fit in the room
    // left, and asserts in debug builds that `String::push` then has room.
    ("push", "len"),
    ("push", "capacity"),
    ("push", "push"),
    // The removed bytes stay in the spare capacity, which `wipe` and the drop
    // overwrite with the rest.
    ("backspace", "pop"),
    ("len", "chars"),
    ("is_empty", "is_empty"),
    // Overwrites the whole allocation, spare capacity included.
    ("wipe", "zeroize"),
];

#[test]
fn typed_ahead_never_logs_or_writes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join(TYPED_AHEAD_SOURCE))
        .unwrap_or_else(|err| panic!("cannot read {TYPED_AHEAD_SOURCE}: {err}"));
    let scan = typed_text_scan(&source)
        .unwrap_or_else(|err| panic!("cannot tokenize {TYPED_AHEAD_SOURCE}: {err}"));
    let leaks: Vec<String> = scan
        .leaks
        .iter()
        .map(|(line, what)| format!("{TYPED_AHEAD_SOURCE}:{line}: {what}"))
        .collect();
    assert!(
        leaks.is_empty(),
        "what is typed into the locked panel could get out of its buffer:\n{}",
        leaks.join("\n")
    );
    // The scan has to see the field it guards, or it proves nothing.
    for known in ["push", "wipe", "take_query"] {
        assert!(
            scan.users.iter().any(|name| name == known),
            "the scan no longer sees `fn {known}` touch the text; found {:?}",
            scan.users
        );
    }
}

#[test]
fn typed_ahead_wipes_by_overwriting() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join(TYPED_AHEAD_SOURCE))
        .unwrap_or_else(|err| panic!("cannot read {TYPED_AHEAD_SOURCE}: {err}"));
    let scan = typed_text_scan(&source)
        .unwrap_or_else(|err| panic!("cannot tokenize {TYPED_AHEAD_SOURCE}: {err}"));
    let mut unused: Vec<(&str, &str)> = TYPED_TEXT_CALLS.to_vec();
    let mut problems = Vec::new();
    for (line, function, what) in &scan.calls {
        if what == "&mut" {
            problems.push(format!(
                "{TYPED_AHEAD_SOURCE}:{line}: `fn {function}` borrows the text mutably, so \
                 whatever it is handed to could shorten or regrow it out of sight"
            ));
        } else if TYPED_TEXT_CALLS.contains(&(function.as_str(), what.as_str())) {
            unused.retain(|&(f, m)| (f, m) != (function.as_str(), what.as_str()));
        } else {
            problems.push(format!(
                "{TYPED_AHEAD_SOURCE}:{line}: `fn {function}` calls `{what}` on the text, and \
                 TYPED_TEXT_CALLS does not list it. Only `zeroize` overwrites what was typed."
            ));
        }
    }
    for (function, method) in unused {
        problems.push(if (function, method) == ("wipe", "zeroize") {
            format!(
                "{TYPED_AHEAD_SOURCE}: `fn wipe` no longer calls `zeroize` on the text; it has \
                 to overwrite what was typed, not only let go of it"
            )
        } else {
            format!(
                "{TYPED_AHEAD_SOURCE}: listed as calling `{method}` on the text in \
                 `fn {function}`, but no such call is left. Remove the line."
            )
        });
    }
    assert!(
        problems.is_empty(),
        "what was typed into the locked panel could stay in memory after a wipe:\n{}",
        problems.join("\n")
    );
}

/// What the scan of the typed-ahead file found.
struct TextScan {
    /// Each place, by line, where the text could get out.
    leaks: Vec<(usize, String)>,
    /// The functions that touch the field `text`.
    users: Vec<String>,
    /// Each method called on the field, by line, with the function it is
    /// called in, and `&mut` for a mutable borrow of it.
    calls: Vec<(usize, String, String)>,
}

/// Reads the typed-ahead file outside items switched off by `#[cfg(test)]`.
/// A name from [`TEXT_OUTLETS`] anywhere is a leak, and so is the field
/// `text` inside a macro call (`format!`, `panic!`, `write!` and the like),
/// in a function not on [`TYPED_TEXT_USERS`], or outside a function other
/// than in the declaration of `struct TypedAhead`. In the functions on that
/// list it notes what is done to the field ([`text_call`]).
fn typed_text_scan(source: &str) -> Result<TextScan, String> {
    let p = parse(source)?;
    let t = &p.tokens;
    let test_items = cfg_test_items(&p);
    let mut scan = TextScan {
        leaks: Vec::new(),
        users: Vec::new(),
        calls: Vec::new(),
    };
    for (i, token) in t.iter().enumerate() {
        if test_items.iter().any(|item| item.contains(&i)) {
            continue;
        }
        let Some(name) = token.name() else {
            continue;
        };
        let line = token.line;
        if TEXT_OUTLETS.contains(&name) {
            scan.leaks.push((line, format!("`{name}`")));
            continue;
        }
        if name != "text" {
            continue;
        }
        if p.in_macro_call[i] {
            scan.leaks
                .push((line, "the text in a macro call".to_string()));
            continue;
        }
        match enclosing_fn(&p, i) {
            Some(function) if TYPED_TEXT_USERS.contains(&function.as_str()) => {
                if let Some(what) = text_call(t, i) {
                    scan.calls.push((line, function.clone(), what));
                }
                if !scan.users.contains(&function) {
                    scan.users.push(function);
                }
            }
            Some(function) => scan.leaks.push((
                line,
                format!("the text in `fn {function}`, which is not on TYPED_TEXT_USERS"),
            )),
            None if p.enclosing_brace[i].is_some_and(|open| {
                anchor(&t[header_start(&p, open)..open]) == "struct TypedAhead"
            }) => {}
            None => scan
                .leaks
                .push((line, "the text outside a function".to_string())),
        }
    }
    Ok(scan)
}

#[test]
fn typed_text_leaks_are_found() {
    let source = r##"
        use std::fs;
        struct TypedAhead { text: Zeroizing<String>, started: SystemTime }
        impl TypedAhead {
            fn push(&mut self, c: char) { self.text.push(c); }
            fn note(&self) { log::info!("{}", self.len()); }
            fn shout(&self) { eprintln!("{}", self.text.as_str()); }
            fn leak(&self) -> String { self.text.to_string() }
            // self.text in a comment, and "text" in a string
            fn save(&self) { std::fs::write("/tmp/x", "y").ok(); }
        }
        impl fmt::Display for TypedAhead {}
        #[derive(Debug)]
        struct Other;
        static COPY: &str = text;
        #[cfg(test)]
        mod tests { fn helper(b: &TypedAhead) { println!("{}", b.text.as_str()); } }
    "##;
    let scan = typed_text_scan(source).expect("scan");
    let lines: Vec<usize> = scan.leaks.iter().map(|(line, _)| *line).collect();
    assert_eq!(
        lines,
        vec![2, 6, 7, 7, 8, 10, 12, 13, 15],
        "{:?}",
        scan.leaks
    );
    assert_eq!(scan.users, vec!["push".to_string()]);
}

/// What the code does to the field `text` at token `i`, where the scan has
/// to know it: the name of a method called on it directly
/// (`self.text.zeroize()`), or `&mut` for a mutable borrow
/// (`&mut self.text`), through which code elsewhere could change it.
fn text_call(t: &[Token], i: usize) -> Option<String> {
    if t.get(i + 1).is_some_and(|tok| tok.is_punct('.'))
        && t.get(i + 3).is_some_and(|tok| tok.is_punct('('))
        && let Some(method) = t.get(i + 2).and_then(Token::name)
    {
        return Some(method.to_string());
    }
    let mut j = i;
    if j >= 2 && t[j - 1].is_punct('.') && t[j - 2].is_keyword("self") {
        j -= 2;
    }
    (j >= 2 && t[j - 1].is_keyword("mut") && t[j - 2].is_punct('&')).then(|| "&mut".to_string())
}

#[test]
fn typed_text_calls_are_found() {
    let source = r##"
        impl TypedAhead {
            fn push(&mut self, c: char) { if self.text.len() < self.text.capacity() { self.text.push(c); } }
            fn wipe(&mut self) { self.text.clear(); self.guard = None; }
            fn backspace(&mut self) { String::truncate(&mut self.text, 0); }
            fn take_query(self) -> Zeroizing<String> { let Self { text, .. } = self; text }
            fn len(&self) -> usize { self.text.chars().count() }
            fn is_empty(&self) -> bool { is_blank(&self.text) }
        }
        #[cfg(test)]
        mod tests { fn wipe(buffer: &mut TypedAhead) { buffer.text.zeroize(); } }
    "##;
    let scan = typed_text_scan(source).expect("scan");
    let calls: Vec<(usize, &str, &str)> = scan
        .calls
        .iter()
        .map(|(line, function, what)| (*line, function.as_str(), what.as_str()))
        .collect();
    assert_eq!(
        calls,
        vec![
            (3, "push", "len"),
            (3, "push", "capacity"),
            (3, "push", "push"),
            (4, "wipe", "clear"),
            (5, "backspace", "&mut"),
            (7, "len", "chars"),
        ]
    );
}

/// The Touch ID flow the lock screen and the quick access panel share.
const TOUCH_ID_FLOW: &str = "src/app/unlock/touch_id.rs";

#[test]
fn the_raw_flow_has_no_public_entry() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join(TOUCH_ID_FLOW))
        .unwrap_or_else(|err| panic!("cannot read {TOUCH_ID_FLOW}: {err}"));
    let problems = raw_flow_entries(&source)
        .unwrap_or_else(|err| panic!("cannot tokenize {TOUCH_ID_FLOW}: {err}"));
    let problems: Vec<String> = problems
        .iter()
        .map(|(line, what)| format!("{TOUCH_ID_FLOW}:{line}: {what}"))
        .collect();
    assert!(
        problems.is_empty(),
        "the quick access panel could start Touch ID without a confirmation:\n{}",
        problems.join("\n")
    );
}

/// What in the Touch ID flow would let code start the prompt for the panel
/// without a token, outside items switched off by `#[cfg(test)]`:
///
/// - `fn run` missing, or written with a visibility;
/// - `struct Confirmed` missing, or with anything but one private `()`;
/// - `Clone`, `Copy` or `Default` derived for it, a trait implemented for it
///   by hand, or methods added to it;
/// - a `Confirmed` built anywhere but in `fn confirm` (taking one apart with
///   `let Confirmed(..) =` is fine);
/// - a function with a visibility, other than `confirm`, that takes an
///   `UnlockOrigin` or a `PanelTrigger`;
/// - `run` called anywhere but in `unlock_from_lock_screen` and
///   `unlock_from_panel`, or named without being called;
/// - the name `Panel` anywhere but in `unlock_from_panel`, in the
///   declaration of `UnlockOrigin`, and on the right of a comparison
///   (`origin == UnlockOrigin::Panel`). A panel origin made anywhere else
///   could reach `run` through a function that takes no token.
fn raw_flow_entries(source: &str) -> Result<Vec<(usize, String)>, String> {
    let p = parse(source)?;
    let t = &p.tokens;
    let test_items = cfg_test_items(&p);
    let mut problems = Vec::new();
    let mut saw_run = false;
    let mut saw_token = false;
    for i in 0..t.len() {
        if test_items.iter().any(|item| item.contains(&i)) {
            continue;
        }
        let line = t[i].line;
        if t[i].is_keyword("fn") {
            let Some(name) = t.get(i + 1).and_then(Token::name) else {
                continue;
            };
            let visibility = visibility_before(&p, i);
            if name == "run" {
                saw_run = true;
                if !visibility.is_empty() {
                    problems.push((
                        line,
                        format!("`fn run` is `{visibility}`; the raw flow has to stay private"),
                    ));
                }
            }
            if visibility.is_empty() || name == "confirm" {
                continue;
            }
            let Some(open) = (i + 2..t.len()).find(|&j| t[j].is_punct('(')) else {
                continue;
            };
            let close = p.partner[open].expect("balanced parenthesis");
            for parameter in &t[open + 1..close] {
                if let Some(kind @ ("UnlockOrigin" | "PanelTrigger")) = parameter.name() {
                    problems.push((
                        line,
                        format!(
                            "`{visibility} fn {name}` takes a `{kind}` as a plain value; only \
                             `confirm` may, and only a `Confirmed` opens the panel's way in"
                        ),
                    ));
                }
            }
        } else if t[i].name() == Some("Confirmed") {
            let before = i.checked_sub(1).map(|j| &t[j]);
            let after = t.get(i + 1);
            if before.is_some_and(|tok| tok.is_keyword("struct")) {
                saw_token = true;
                let body: Vec<&Tok> = t[i + 1..].iter().take(5).map(|tok| &tok.tok).collect();
                let private_unit = [
                    Tok::Punct('('),
                    Tok::Punct('('),
                    Tok::Punct(')'),
                    Tok::Punct(')'),
                    Tok::Punct(';'),
                ];
                if body.len() != private_unit.len()
                    || body.iter().zip(&private_unit).any(|(a, b)| *a != b)
                {
                    problems.push((
                        line,
                        "`struct Confirmed` has to be `Confirmed(())`, one private field"
                            .to_string(),
                    ));
                }
                for derived in derives_before(&p, i - 1) {
                    if matches!(derived.as_str(), "Clone" | "Copy" | "Default") {
                        problems.push((
                            line,
                            format!("`Confirmed` derives `{derived}`; a token is used up once"),
                        ));
                    }
                }
            } else if before.is_some_and(|tok| tok.is_keyword("for")) {
                problems.push((
                    line,
                    "implements a trait for `Confirmed` by hand".to_string(),
                ));
            } else if before.is_some_and(|tok| tok.is_keyword("impl")) {
                problems.push((
                    line,
                    "adds methods to `Confirmed`, which could hand out tokens".to_string(),
                ));
            } else if after.is_some_and(|tok| tok.is_punct('(') || tok.is_punct('{')) {
                let taken_apart = before.is_some_and(|tok| tok.is_keyword("let"));
                let function = enclosing_fn(&p, i);
                if !taken_apart && function.as_deref() != Some("confirm") {
                    problems.push((
                        line,
                        format!(
                            "builds a `Confirmed` in {}; only `fn confirm` may",
                            function.map_or("no function".to_string(), |f| format!("`fn {f}`"))
                        ),
                    ));
                }
            }
        } else if t[i].name() == Some("run") {
            // The definition is checked above; `.run(` is a method of
            // something else.
            if i > 0 && (t[i - 1].is_keyword("fn") || t[i - 1].is_punct('.')) {
                continue;
            }
            let function = enclosing_fn(&p, i);
            if !t.get(i + 1).is_some_and(|tok| tok.is_punct('(')) {
                problems.push((
                    line,
                    "`run` is named without being called, so a call could go unseen".to_string(),
                ));
            } else if !matches!(
                function.as_deref(),
                Some("unlock_from_lock_screen" | "unlock_from_panel")
            ) {
                problems.push((
                    line,
                    format!(
                        "calls `run` in {}; only `unlock_from_lock_screen` and \
                         `unlock_from_panel` may, so the panel's way in keeps its token",
                        function.map_or("no function".to_string(), |f| format!("`fn {f}`"))
                    ),
                ));
            }
        } else if t[i].name() == Some("Panel") {
            let function = enclosing_fn(&p, i);
            let declared = function.is_none()
                && p.enclosing_brace[i].is_some_and(|open| {
                    anchor(&t[header_start(&p, open)..open]) == "enum UnlockOrigin"
                });
            let compared = i >= 5
                && t[i - 1].is_punct(':')
                && t[i - 2].is_punct(':')
                && t[i - 3].name() == Some("UnlockOrigin")
                && t[i - 4].is_punct('=')
                && (t[i - 5].is_punct('=') || t[i - 5].is_punct('!'));
            if !declared && !compared && function.as_deref() != Some("unlock_from_panel") {
                problems.push((
                    line,
                    format!(
                        "names the `Panel` origin in {}; only `fn unlock_from_panel` may make \
                         one, behind its token",
                        function.map_or("no function".to_string(), |f| format!("`fn {f}`"))
                    ),
                ));
            }
        }
    }
    if !saw_run {
        problems.push((0, "there is no `fn run` to check".to_string()));
    }
    if !saw_token {
        problems.push((0, "there is no `struct Confirmed` to check".to_string()));
    }
    Ok(problems)
}

/// The visibility written before the item keyword at token `i` (`pub`,
/// `pub(crate)`, `pub(super)` and so on), or an empty string for none.
/// Qualifiers such as `const` or `async` in between are skipped.
fn visibility_before(p: &Parsed, i: usize) -> String {
    let t = &p.tokens;
    let mut j = i;
    while j > 0
        && (matches!(
            t[j - 1].name(),
            Some("const" | "async" | "unsafe" | "safe" | "extern")
        ) || matches!(t[j - 1].tok, Tok::Str(_)))
    {
        j -= 1;
    }
    if j > 0 && t[j - 1].is_punct(')') {
        let open = p.partner[j - 1].expect("balanced parenthesis");
        if open > 0 && t[open - 1].is_keyword("pub") {
            let scope: Vec<&str> = t[open + 1..j - 1].iter().filter_map(Token::name).collect();
            return format!("pub({})", scope.join(" "));
        }
    }
    if j > 0 && t[j - 1].is_keyword("pub") {
        return "pub".to_string();
    }
    String::new()
}

/// The names in the `derive` attributes above the item whose keyword is at
/// token `i`.
fn derives_before(p: &Parsed, i: usize) -> Vec<String> {
    let t = &p.tokens;
    let mut j = i;
    if j > 0 && t[j - 1].is_punct(')') {
        j = p.partner[j - 1].expect("balanced parenthesis");
    }
    if j > 0 && t[j - 1].is_keyword("pub") {
        j -= 1;
    }
    let mut names = Vec::new();
    while j > 0 && t[j - 1].is_punct(']') {
        let open = p.partner[j - 1].expect("balanced bracket");
        let inside = &t[open + 1..j - 1];
        if inside.first().is_some_and(|tok| tok.is_keyword("derive")) {
            names.extend(
                inside
                    .iter()
                    .skip(1)
                    .filter_map(Token::name)
                    .map(str::to_string),
            );
        }
        j = open.saturating_sub(1);
    }
    names
}

#[test]
fn raw_flow_entries_are_found() {
    let fine = r##"
        #[derive(Clone, Copy)]
        pub enum UnlockOrigin { LockScreen, Panel }
        #[derive(Debug)]
        #[must_use]
        pub struct Confirmed(());
        pub fn confirm(trigger: PanelTrigger) -> Option<Confirmed> { Some(Confirmed(())) }
        pub fn unlock_from_lock_screen(cx: &mut App) -> Option<u64> {
            run(UnlockOrigin::LockScreen, cx)
        }
        pub fn unlock_from_panel(token: Confirmed, cx: &mut App) -> Option<u64> {
            let Confirmed(()) = token;
            run(UnlockOrigin::Panel, cx)
        }
        fn run(origin: UnlockOrigin, cx: &mut App) -> Option<u64> {
            if origin == UnlockOrigin::Panel { return None; }
            cx.background_executor().run(async {});
            None
        }
        #[cfg(test)]
        mod tests {
            pub fn sneak() -> Confirmed { Confirmed(()) }
            fn stale(cx: &mut App) { run(UnlockOrigin::Panel, cx); }
        }
    "##;
    assert_eq!(raw_flow_entries(fine).expect("scan"), Vec::new());

    let broken = r##"
        #[derive(Debug, Clone)]
        pub struct Confirmed(pub ());
        impl Default for Confirmed { fn default() -> Self { Self(()) } }
        impl Confirmed { pub fn again(&self) -> Self { Confirmed(()) } }
        pub fn unlock_from_panel(trigger: PanelTrigger) -> Option<u64> { None }
        pub(crate) fn run(origin: UnlockOrigin) -> Option<u64> {
            let token = Confirmed(());
            None
        }
    "##;
    let lines: Vec<usize> = raw_flow_entries(broken)
        .expect("scan")
        .into_iter()
        .map(|(line, _)| line)
        .collect();
    assert_eq!(lines, vec![3, 3, 4, 5, 5, 6, 7, 7, 8]);

    // Ways past the token that take no origin as a parameter: a new entry
    // that calls the flow, the flow passed as a value, a panel origin made
    // in a helper, renamed by a `use`, or matched rather than compared.
    let around = r##"
        pub enum UnlockOrigin { LockScreen, Panel }
        pub struct Confirmed(());
        fn run(origin: UnlockOrigin, cx: &mut App) -> Option<u64> { None }
        pub fn unlock_from_panel_at_once(cx: &mut App) -> Option<u64> {
            run(UnlockOrigin::Panel, cx)
        }
        pub fn unlock_later(cx: &mut App) { let flow = run; }
        fn origin_for_timer() -> UnlockOrigin { UnlockOrigin::Panel }
        use self::UnlockOrigin::Panel as FromPanel;
        fn on_timer(origin: UnlockOrigin) { if matches!(origin, UnlockOrigin::Panel) {} }
    "##;
    let lines: Vec<usize> = raw_flow_entries(around)
        .expect("scan")
        .into_iter()
        .map(|(line, _)| line)
        .collect();
    assert_eq!(lines, vec![6, 6, 8, 9, 10, 11]);

    assert_eq!(
        raw_flow_entries("fn other() {}").expect("scan"),
        vec![
            (0, "there is no `fn run` to check".to_string()),
            (0, "there is no `struct Confirmed` to check".to_string()),
        ]
    );
}

/// The shell file with the lock screen's Touch ID.
const LOCK_SCREEN_SOURCE: &str = "src/ui/legacy/shell/unlock.rs";

/// The functions there that raise the automatic prompt or run an attempt on
/// their own. None of them may be visible beyond the shell: the lock screen's
/// automatic prompt has no way in from anywhere else.
const LOCK_SCREEN_PROMPTS: &[&str] = &[
    "schedule_automatic_biometric_unlock",
    "start_automatic_biometric_unlock",
    "start_biometric_unlock",
    "note_automatic_biometric_interruption",
];

#[test]
fn the_automatic_prompt_has_no_public_entry() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join(LOCK_SCREEN_SOURCE))
        .unwrap_or_else(|err| panic!("cannot read {LOCK_SCREEN_SOURCE}: {err}"));
    let found = function_visibility(&source)
        .unwrap_or_else(|err| panic!("cannot tokenize {LOCK_SCREEN_SOURCE}: {err}"));
    let mut problems = Vec::new();
    for name in LOCK_SCREEN_PROMPTS {
        match found.iter().find(|(function, _)| function == name) {
            None => problems.push(format!("there is no `fn {name}` to check")),
            Some((_, visibility)) if !matches!(visibility.as_str(), "" | "pub(super)") => problems
                .push(format!(
                    "`fn {name}` is `{visibility}`; the automatic prompt stays inside the shell"
                )),
            Some(_) => {}
        }
    }
    assert!(
        problems.is_empty(),
        "{LOCK_SCREEN_SOURCE}:\n{}",
        problems.join("\n")
    );
}

/// Every function in code outside `#[cfg(test)]` items, with the visibility
/// written before it.
fn function_visibility(source: &str) -> Result<Vec<(String, String)>, String> {
    let p = parse(source)?;
    let t = &p.tokens;
    let test_items = cfg_test_items(&p);
    Ok((0..t.len())
        .filter(|&i| t[i].is_keyword("fn") && !test_items.iter().any(|item| item.contains(&i)))
        .filter_map(|i| {
            let name = t.get(i + 1)?.name()?;
            Some((name.to_string(), visibility_before(&p, i)))
        })
        .collect())
}

#[test]
fn function_visibility_is_read_as_written() {
    let source = r##"
        impl Shell {
            fn private(&self) {}
            pub(super) fn shell_only(&mut self) {}
            pub(crate) async fn crate_wide(&self) {}
            pub fn everyone(&self) {}
            pub const fn constant() -> u8 { 0 }
        }
        #[cfg(test)]
        mod tests { pub fn helper() {} }
    "##;
    let pair = |name: &str, visibility: &str| (name.to_string(), visibility.to_string());
    assert_eq!(
        function_visibility(source).expect("scan"),
        vec![
            pair("private", ""),
            pair("shell_only", "pub(super)"),
            pair("crate_wide", "pub(crate)"),
            pair("everyone", "pub"),
            pair("constant", "pub"),
        ]
    );
}

#[test]
fn activity_is_touched_only_where_listed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sources: Vec<(String, String)> = rust_sources(root)
        .into_iter()
        .map(|(rel, path)| {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
            (rel, source)
        })
        .collect();
    let test_only = test_only_files(&sources).unwrap_or_else(|err| panic!("cannot tokenize {err}"));
    let mut touches = Vec::new();
    let mut resets = Vec::new();
    let mut problems = Vec::new();
    for (rel, source) in &sources {
        if test_only.contains(rel) {
            continue;
        }
        let calls =
            activity_calls(source).unwrap_or_else(|err| panic!("cannot tokenize {rel}: {err}"));
        for (line, call) in calls {
            match call {
                Ok(ClockCall::Touch(named)) => touches.push((rel.as_str(), named, line)),
                Ok(ClockCall::Reset(function)) => resets.push((rel.as_str(), function, line)),
                Err(problem) => problems.push(format!("{rel}:{line}: {problem}")),
            }
        }
    }

    for (file, source, line) in &touches {
        if !ACTIVITY_CALL_SITES.contains(&(*file, source.as_str())) {
            problems.push(format!(
                "{file}:{line}: touches the idle clock as `{source}`, which is not on the \
                 list. Only input the user gives FerrisPass holds off the auto-lock; if this \
                 is such input, add the file and source to ACTIVITY_CALL_SITES."
            ));
        }
    }
    for (file, source) in ACTIVITY_CALL_SITES {
        if !touches.iter().any(|(f, s, _)| f == file && s == source) {
            problems.push(format!(
                "{file}: listed as touching the idle clock as `{source}`, but no such call \
                 is left. Remove the entry."
            ));
        }
    }

    // Each line on the list allows one call.
    let mut unused: Vec<(&str, &str)> = RESET_CALL_SITES.to_vec();
    for (file, function, line) in &resets {
        match unused
            .iter()
            .position(|&(f, func)| f == *file && func == function.as_str())
        {
            Some(at) => {
                unused.remove(at);
            }
            None => problems.push(format!(
                "{file}:{line}: resets the idle clock in `fn {function}`, and \
                 RESET_CALL_SITES has no line left for it. A reset holds off the auto-lock \
                 just as input does; only the auto-lock task sets the baseline, when it starts."
            )),
        }
    }
    for (file, function) in unused {
        problems.push(format!(
            "{file}: listed as resetting the idle clock in `fn {function}`, but no such call \
             is left. Remove the line."
        ));
    }
    assert!(
        problems.is_empty(),
        "the idle clock is set outside the lists of call sites:\n{}",
        problems.join("\n")
    );
}

/// Files the crate builds only for its own tests: the file of each
/// `#[cfg(test)] mod name;` at the top level of a source file, and every
/// module below it. They count as test code, as an inline `#[cfg(test)] mod`
/// does. A declaration this does not place, one with `#[path]` or inside an
/// inline module, leaves its file to the scan, which errs on the side of a
/// finding.
fn test_only_files(sources: &[(String, String)]) -> Result<Vec<String>, String> {
    let mut modules = Vec::new();
    for (rel, source) in sources {
        let p = parse(source).map_err(|err| format!("{rel}: {err}"))?;
        let t = &p.tokens;
        let (dir, file) = rel.rsplit_once('/').unwrap_or(("", rel));
        let stem = file.strip_suffix(".rs").unwrap_or(file);
        let base = if matches!(stem, "mod" | "lib" | "main") {
            dir.to_string()
        } else {
            format!("{dir}/{stem}")
        };
        for item in cfg_test_items(&p) {
            let (start, end) = (*item.start(), *item.end());
            let declares_a_file = p.enclosing_brace[start].is_none()
                && end >= 2
                && t[end].is_punct(';')
                && t[end - 2].is_keyword("mod")
                && !t[start..end].iter().any(|tok| tok.is_keyword("path"));
            if !declares_a_file {
                continue;
            }
            if let Some(name) = t[end - 1].name() {
                modules.push(format!("{base}/{name}"));
            }
        }
    }
    Ok(sources
        .iter()
        .map(|(rel, _)| rel)
        .filter(|rel| {
            modules.iter().any(|module| {
                rel.strip_prefix(module.as_str())
                    .is_some_and(|rest| rest == ".rs" || rest.starts_with('/'))
            })
        })
        .cloned()
        .collect())
}

/// What a call into the activity module does to the idle clock.
#[derive(Debug, PartialEq)]
enum ClockCall {
    /// `activity::touch`, with the source it names.
    Touch(String),
    /// `activity::reset`, with the function it is in.
    Reset(String),
}

/// A line, and the call found there or what is wrong with it.
type FoundCall = (usize, Result<ClockCall, String>);

/// Each `activity::touch` and `activity::reset` in code, with its line,
/// outside items switched off by `#[cfg(test)]`: `Ok` with the source a touch
/// names or the function a reset is in, or a problem. Ways a call could go
/// by another name are problems too: `touch` or `reset` imported or used as a
/// value, a glob import from the module, the module renamed with `as`.
fn activity_calls(source: &str) -> Result<Vec<FoundCall>, String> {
    let p = parse(source)?;
    let t = &p.tokens;
    let test_items = cfg_test_items(&p);
    let path_sep = |i: usize| {
        t.get(i).is_some_and(|tok| tok.is_punct(':'))
            && t.get(i + 1).is_some_and(|tok| tok.is_punct(':'))
    };
    let setter = |tok: &Token| matches!(tok.name(), Some("touch" | "reset"));
    let mut found = Vec::new();
    for i in 0..t.len() {
        if t[i].name() != Some("activity") || test_items.iter().any(|item| item.contains(&i)) {
            continue;
        }
        let line = t[i].line;
        if t.get(i + 1).is_some_and(|tok| tok.is_keyword("as")) {
            found.push((
                line,
                Err("renames the activity module, so its calls go unseen".to_string()),
            ));
            continue;
        }
        if !path_sep(i + 1) {
            continue;
        }
        let Some(next) = t.get(i + 3) else {
            continue;
        };
        if let Some(name) = next.name().filter(|_| setter(next)) {
            if !t.get(i + 4).is_some_and(|tok| tok.is_punct('(')) {
                let arguments = if name == "touch" {
                    "ActivitySource::.."
                } else {
                    ".."
                };
                found.push((
                    line,
                    Err(format!(
                        "`activity::{name}` is imported or passed around; call it as \
                         `activity::{name}({arguments})`"
                    )),
                ));
                continue;
            }
            let call = if name == "touch" {
                let close = p.partner[i + 4].expect("balanced parenthesis");
                named_source(&t[i + 5..close]).map(ClockCall::Touch)
            } else {
                enclosing_fn(&p, i)
                    .map(ClockCall::Reset)
                    .ok_or_else(|| "calls `activity::reset` outside a function".to_string())
            };
            found.push((line, call));
        } else if next.is_punct('*') {
            found.push((
                line,
                Err("glob import from the activity module".to_string()),
            ));
        } else if next.is_punct('{') {
            let close = p.partner[i + 3].expect("balanced brace");
            let group = &t[i + 4..close];
            if group
                .iter()
                .any(|tok| setter(tok) || tok.is_punct('*') || tok.is_keyword("as"))
            {
                found.push((
                    line,
                    Err(
                        "imports `touch` or `reset` from the activity module, or renames \
                         something from it"
                            .to_string(),
                    ),
                ));
            }
        }
    }
    Ok(found)
}

/// The name of the innermost `fn` whose body holds token `i`. Closures and
/// blocks in between don't count, so a call in a task a function spawns
/// belongs to that function.
fn enclosing_fn(p: &Parsed, i: usize) -> Option<String> {
    let mut brace = p.enclosing_brace[i];
    while let Some(open) = brace {
        let header = anchor(&p.tokens[header_start(p, open)..open]);
        if let Some(name) = header.strip_prefix("fn ") {
            return Some(name.to_string());
        }
        brace = p.enclosing_brace[open];
    }
    None
}

/// The source a call of `activity::touch` names. The argument has to be
/// `ActivitySource::<name>`, with or without a path in front, so the call
/// site says which source it is.
fn named_source(argument: &[Token]) -> Result<String, String> {
    let unnamed = || {
        Err("calls `activity::touch` without naming the source; pass \
             `ActivitySource::<name>` directly"
            .to_string())
    };
    let Some(split) = argument.len().checked_sub(4) else {
        return unnamed();
    };
    let (path, tail) = argument.split_at(split);
    let path_is_plain = path.len() % 3 == 0
        && path.chunks(3).all(|segment| {
            segment[0].name().is_some() && segment[1].is_punct(':') && segment[2].is_punct(':')
        });
    match tail {
        [ty, colon, colon2, variant]
            if path_is_plain
                && ty.name() == Some("ActivitySource")
                && colon.is_punct(':')
                && colon2.is_punct(':') =>
        {
            match variant.name() {
                Some(name) => Ok(name.to_string()),
                None => unnamed(),
            }
        }
        _ => unnamed(),
    }
}

#[test]
fn activity_touches_are_found_with_their_source() {
    let source = r##"
        use crate::app::services::activity::{self, ActivitySource};
        // activity::touch(ActivitySource::Timer) in a comment
        fn input() { activity::touch(ActivitySource::WindowInput); }
        const NOTE: &str = "activity::touch(ActivitySource::Timer)";
        fn full() { crate::app::services::activity::touch(crate::app::services::activity::ActivitySource::Panel); }
        fn forwarded(source: ActivitySource) { activity::touch(source); }
        #[cfg(test)]
        mod tests { fn helper() { activity::touch(ActivitySource::Hotkey); } }
        fn reads() { let _ = activity::idle_for(now); let _ = activity::lock_is_due(idle, None); }
        use crate::app::services::activity::touch;
        use crate::app::services::activity::{touch as poke};
        use crate::app::services::activity::*;
        use crate::app::services::activity as clock;
        fn pointer() { let f = activity::touch; }
    "##;
    let found = activity_calls(source).expect("scan");
    let named: Vec<(usize, String)> = found
        .iter()
        .filter_map(|(line, call)| match call {
            Ok(ClockCall::Touch(source)) => Some((*line, source.clone())),
            _ => None,
        })
        .collect();
    let problems: Vec<usize> = found
        .iter()
        .filter(|(_, call)| call.is_err())
        .map(|(line, _)| *line)
        .collect();
    assert_eq!(
        named,
        vec![(4, "WindowInput".to_string()), (6, "Panel".to_string())]
    );
    assert_eq!(problems, vec![7, 11, 12, 13, 14, 15]);
    assert_eq!(found.len(), named.len() + problems.len(), "{found:?}");
}

#[test]
fn activity_resets_are_found_with_their_function() {
    let source = r##"
        use crate::app::services::activity::{self, ActivitySource};
        // activity::reset(now) in a comment
        fn start() { activity::reset(SystemTime::now()); }
        const NOTE: &str = "activity::reset(now)";
        impl Shell { fn task(&mut self) { spawn(async move |_| { loop { activity::reset(now); } }); } }
        fn outer() { let f = || { fn inner() { crate::app::services::activity::reset(now); } }; }
        #[cfg(test)]
        mod tests { fn helper() { activity::reset(now); } }
        const _: () = { activity::reset(now); };
        use crate::app::services::activity::reset;
        use crate::app::services::activity::{self, reset};
        fn pointer() { let f = activity::reset; }
    "##;
    assert_eq!(
        activity_calls(source).expect("scan"),
        vec![
            (4, Ok(ClockCall::Reset("start".to_string()))),
            (6, Ok(ClockCall::Reset("task".to_string()))),
            (7, Ok(ClockCall::Reset("inner".to_string()))),
            (
                10,
                Err("calls `activity::reset` outside a function".to_string())
            ),
            (
                11,
                Err(
                    "`activity::reset` is imported or passed around; call it as \
                     `activity::reset(..)`"
                        .to_string()
                )
            ),
            (
                12,
                Err(
                    "imports `touch` or `reset` from the activity module, or renames \
                     something from it"
                        .to_string()
                )
            ),
            (
                13,
                Err(
                    "`activity::reset` is imported or passed around; call it as \
                     `activity::reset(..)`"
                        .to_string()
                )
            ),
        ]
    );
}

#[test]
fn files_under_a_test_only_module_are_found() {
    let file = |rel: &str, source: &str| (rel.to_string(), source.to_string());
    let sources = [
        file(
            "src/shell/mod.rs",
            "#[cfg(test)]\n#[allow(dead_code)]\nmod clock_tests;\nmod render;\n\
             #[cfg(test)]\n#[path = \"moved_here.rs\"]\nmod moved;\n\
             mod inline { #[cfg(test)] mod nested; }\n#[cfg(not(test))]\nmod live;",
        ),
        file("src/shell/clock_tests.rs", "mod helpers;"),
        file("src/shell/clock_tests/helpers.rs", ""),
        file("src/shell/clock_tests_more.rs", ""),
        file("src/shell/render.rs", "#[cfg(test)] mod render_tests;"),
        file("src/shell/render/render_tests/mod.rs", ""),
        file("src/shell/moved.rs", ""),
        file("src/shell/moved_here.rs", ""),
        file("src/shell/nested.rs", ""),
        file("src/shell/live.rs", ""),
    ];
    assert_eq!(
        test_only_files(&sources).expect("scan"),
        vec![
            "src/shell/clock_tests.rs",
            "src/shell/clock_tests/helpers.rs",
            "src/shell/render/render_tests/mod.rs",
        ]
    );
}

// -------------------------------------------------------- release pipeline --

/// The release rules, read from the workflow files as text, because nothing
/// else exercises a workflow before a tag does.
///
/// Every release is published as a pre-release, and the workflow has no other
/// way to create a release, edit one or mark one as latest; the publishing
/// step says `make_latest: false` outright. The updater reads
/// `releases/latest`, which GitHub never resolves to a pre-release, so
/// shipping a version to everyone stays a step taken by hand. The signed
/// artefacts the sign job hands over, `SHA256SUMS` included, are all
/// attached, and a missing one stops the publishing step. After
/// publishing, a job of its own, one run at a time and for every tag, runs
/// `scripts/update-channel.sh`, which points the permanent `kanal-vorab`
/// release at the highest version released so far; what that script does
/// is tested by running it, in `tests/release_channel.rs`. Versions are
/// never ordered with `sort -V`, in the workflow or in that script: it
/// ranks `0.10.0-pre.1` above `0.10.0`. No step or job may fail quietly.
///
/// CI and CodeQL run for pushes to `master` and `rebuild` and for pull
/// requests into either, so merges from `master` into the long-lived
/// `rebuild` branch are built and scanned like everything else.
#[test]
fn release_pipeline_rules_hold() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |name: &str| {
        fs::read_to_string(root.join(name))
            .unwrap_or_else(|err| panic!("cannot read {name}: {err}"))
    };

    let mut problems: Vec<String> =
        release_workflow_problems(&read(".github/workflows/release.yml"))
            .into_iter()
            .map(|problem| format!(".github/workflows/release.yml: {problem}"))
            .collect();
    let script = read(CHANNEL_SCRIPT);
    let code: Vec<&str> = script
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .flat_map(str::split_whitespace)
        .collect();
    if let Some(sort) = version_sort(&code) {
        problems.push(format!(
            "{CHANNEL_SCRIPT}: `{sort}` does not order versions by SemVer precedence"
        ));
    }
    for name in [".github/workflows/ci.yml", ".github/workflows/codeql.yml"] {
        let workflow = read(name);
        for event in ["push", "pull_request"] {
            match branch_filter(&workflow, event) {
                Ok(None) => {}
                Ok(Some(branches)) => {
                    for branch in ["master", "rebuild"] {
                        if !branches.iter().any(|b| b == branch) {
                            problems.push(format!(
                                "{name}: `{event}` does not run for {branch}, only for \
                                 {branches:?}"
                            ));
                        }
                    }
                }
                Err(err) => problems.push(format!("{name}: {err}")),
            }
        }
    }
    assert!(
        problems.is_empty(),
        "the release pipeline rules are broken:\n{}",
        problems.join("\n")
    );
}

const PUBLISH_ACTION: &str = "softprops/action-gh-release";
const CHANNEL_RELEASE: &str = "kanal-vorab";
const VERSION_COMPARE: &str = "scripts/semver-compare.py";
const CHANNEL_SCRIPT: &str = "scripts/update-channel.sh";
/// What the sign job hands over in `signed-release` and the release carries.
const RELEASE_FILES: &[&str] = &[
    "dist/*.dmg",
    "dist/*.tar.gz",
    "dist/*.minisig",
    "dist/update.json",
    "dist/SHA256SUMS",
];

fn release_workflow_problems(workflow: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for (_, line) in yaml_lines(workflow) {
        let Some((key, value)) = yaml_entry(line) else {
            continue;
        };
        match key {
            "prerelease" if unquote(value) != "true" => {
                problems.push(format!("`{line}`: every release is a pre-release"));
            }
            "make_latest" if unquote(value) != "false" => {
                problems.push(format!("`{line}`: only a person marks a release as latest"));
            }
            "continue-on-error" => {
                problems.push(format!(
                    "`{line}`: a failing release step must fail the run"
                ));
            }
            _ => {}
        }
    }

    let steps = workflow_steps(workflow);
    let publishing: Vec<usize> = (0..steps.len())
        .filter(|&i| {
            steps[i]
                .get("uses")
                .is_some_and(|uses| uses.starts_with(PUBLISH_ACTION))
        })
        .collect();
    let publish = match publishing[..] {
        [publish] => publish,
        _ => {
            problems.push(format!(
                "expected one step that uses {PUBLISH_ACTION}, found {}",
                publishing.len()
            ));
            return problems;
        }
    };
    for (key, value) in [("prerelease", "true"), ("make_latest", "false")] {
        if !steps[publish]
            .lines
            .iter()
            .any(|line| yaml_entry(line).is_some_and(|(k, _)| k == key))
        {
            problems.push(format!(
                "the step that uses {PUBLISH_ACTION} must set `{key}: {value}`"
            ));
        }
    }
    if !steps[publish].lines.iter().any(|line| {
        yaml_entry(line).is_some_and(|(key, value)| {
            key == "fail_on_unmatched_files" && unquote(value) == "true"
        })
    }) {
        problems.push(format!(
            "the step that uses {PUBLISH_ACTION} must set `fail_on_unmatched_files: true`"
        ));
    }
    let handover: Vec<&Step> = steps
        .iter()
        .filter(|step| {
            step.get("uses")
                .is_some_and(|uses| uses.starts_with("actions/upload-artifact"))
                && step.words().contains(&"signed-release")
        })
        .collect();
    if handover.len() != 1 {
        problems.push(format!(
            "expected one upload of the `signed-release` artefact, found {}",
            handover.len()
        ));
    }
    for file in RELEASE_FILES {
        if !steps[publish].words().contains(file) {
            problems.push(format!("the release does not attach `{file}`"));
        }
        for step in &handover {
            if !step.words().contains(file) {
                problems.push(format!("`signed-release` does not carry `{file}`"));
            }
        }
    }

    let channel: Vec<usize> = (0..steps.len())
        .filter(|&i| steps[i].words().contains(&CHANNEL_SCRIPT))
        .collect();
    match channel[..] {
        [at] => {
            let step = &steps[at];
            let lines = yaml_lines(workflow);
            let job = job_block(&lines, &step.job).unwrap_or_default();
            let key = |name: &str| {
                entries(job).find(|(_, (_, line))| yaml_entry(line).is_some_and(|(k, _)| k == name))
            };
            let needs = key("needs").map_or_else(Vec::new, |(at, (_, line))| {
                yaml_list(yaml_entry(line).map_or("", |(_, v)| v), children(job, at))
            });
            if !needs.contains(&steps[publish].job) {
                problems.push(format!(
                    "{CHANNEL_SCRIPT} runs in a job that needs `{}`, after the release is up",
                    steps[publish].job
                ));
            }
            let concurrency: Vec<(&str, &str)> =
                key("concurrency").map_or_else(Vec::new, |(at, _)| {
                    entries(children(job, at))
                        .filter_map(|(_, (_, line))| yaml_entry(line))
                        .map(|(k, v)| (k, unquote(v)))
                        .collect()
                });
            if !concurrency.contains(&("group", CHANNEL_RELEASE))
                || !concurrency.contains(&("cancel-in-progress", "false"))
            {
                problems.push(format!(
                    "job {} must run alone: `concurrency` with `group: {CHANNEL_RELEASE}` and \
                     `cancel-in-progress: false`",
                    step.job
                ));
            }
            if let Some((_, (_, line))) = key("if") {
                problems.push(format!(
                    "job {}: {CHANNEL_SCRIPT} runs for every tag, not `{line}`",
                    step.job
                ));
            }
            if let Some(condition) = step.get("if") {
                problems.push(format!(
                    "{CHANNEL_SCRIPT} runs for every tag, not `if: {condition}`"
                ));
            }
            if !steps[..at].iter().any(|before| {
                before.job == step.job
                    && before
                        .get("uses")
                        .is_some_and(|uses| uses.starts_with("actions/checkout"))
            }) {
                problems.push(format!(
                    "job {} must check out the repository before it runs {CHANNEL_SCRIPT}",
                    step.job
                ));
            }
        }
        _ => problems.push(format!(
            "expected one step that runs {CHANNEL_SCRIPT}, found {}",
            channel.len()
        )),
    }

    for step in &steps {
        let words = step.words();
        if let Some(sort) = version_sort(&words) {
            problems.push(format!(
                "job {}: `{sort}` does not order versions by SemVer precedence; use \
                 {VERSION_COMPARE}",
                step.job
            ));
        }
        for command in words.windows(3) {
            if command[..2] == ["gh", "release"]
                && matches!(command[2], "create" | "edit" | "delete")
            {
                problems.push(format!(
                    "job {}: `gh release {}`: the workflow publishes through {PUBLISH_ACTION} \
                     only; releases are edited by hand",
                    step.job, command[2]
                ));
            }
        }
        if words
            .windows(4)
            .any(|command| command == ["gh", "release", "upload", CHANNEL_RELEASE])
        {
            problems.push(format!(
                "job {}: only {CHANNEL_SCRIPT} writes to {CHANNEL_RELEASE}",
                step.job
            ));
        }
    }
    problems
}

/// A `sort` that orders by version (`-V`, `--version-sort`), as the command
/// reads, or `None`. It ranks `0.10.0-pre.1` above `0.10.0`.
fn version_sort(words: &[&str]) -> Option<String> {
    words
        .windows(2)
        .find(|pair| {
            let flag = pair[1];
            pair[0] == "sort"
                && (flag == "--version-sort"
                    || (flag.starts_with('-') && !flag.starts_with("--") && flag.contains('V')))
        })
        .map(|pair| pair.join(" "))
}

/// The lines below job `name`'s key, or `None` when the workflow has no
/// such job.
fn job_block<'a>(lines: &'a [(usize, &'a str)], name: &str) -> Option<&'a [(usize, &'a str)]> {
    let jobs = children(lines, top_level(lines, "jobs")?);
    let (at, _) = entries(jobs)
        .find(|(_, (_, line))| yaml_entry(line).is_some_and(|(key, _)| key == name))?;
    Some(children(jobs, at))
}

/// The branches `event` runs for, from the workflow's `on:` block: `None` for
/// every branch, otherwise the names listed under `branches`. A trigger that
/// lists only tags runs for no branch at all. Forms this reader does not
/// follow are an error, not a guess.
fn branch_filter(workflow: &str, event: &str) -> Result<Option<Vec<String>>, String> {
    let lines = yaml_lines(workflow);
    let on = top_level(&lines, "on").ok_or("no `on:` block")?;
    if !yaml_entry(lines[on].1).is_some_and(|(_, value)| value.is_empty()) {
        return Err(format!("`{}`: list the triggers as a block", lines[on].1));
    }
    let triggers = children(&lines, on);
    let (at, (_, line)) = entries(triggers)
        .find(|(_, (_, line))| yaml_entry(line).is_some_and(|(key, _)| key == event))
        .ok_or_else(|| format!("no `{event}` trigger"))?;
    if yaml_entry(line).is_some_and(|(_, value)| !value.is_empty()) {
        return Err(format!("`{line}`: write the filters as a block"));
    }
    let filters = children(triggers, at);
    let mut branches = None;
    let mut tags_only = false;
    for (at, (_, line)) in entries(filters) {
        match yaml_entry(line) {
            Some(("branches", value)) => branches = Some(yaml_list(value, children(filters, at))),
            Some(("tags" | "tags-ignore", _)) => tags_only = true,
            Some(("branches-ignore", _)) => {
                return Err(format!("`{line}`: name the branches instead"));
            }
            _ => {}
        }
    }
    Ok(branches.or(tags_only.then(Vec::new)))
}

/// A list written inline (`[a, b]`), as `- a` lines below its key, or as a
/// single value.
fn yaml_list(value: &str, below: &[(usize, &str)]) -> Vec<String> {
    if let Some(inline) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        return inline
            .split(',')
            .map(|item| unquote(item).to_string())
            .filter(|item| !item.is_empty())
            .collect();
    }
    if !value.is_empty() {
        return vec![unquote(value).to_string()];
    }
    below
        .iter()
        .filter_map(|(_, line)| line.strip_prefix('-'))
        .map(|item| unquote(item).to_string())
        .collect()
}

/// One step of a job: its top-level keys, and all its lines for searching.
struct Step {
    job: String,
    keys: Vec<(String, String)>,
    lines: Vec<String>,
}

impl Step {
    fn get(&self, key: &str) -> Option<&str> {
        self.keys
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, value)| value.as_str())
    }

    /// The step's text split on blanks, with shell line continuations gone,
    /// so a command reads the same however it is wrapped.
    fn words(&self) -> Vec<&str> {
        self.lines
            .iter()
            .flat_map(|line| line.split_whitespace())
            .filter(|word| *word != "\\")
            .collect()
    }
}

/// Every step of every job, in file order.
fn workflow_steps(workflow: &str) -> Vec<Step> {
    let lines = yaml_lines(workflow);
    let mut steps = Vec::new();
    let Some(jobs) = top_level(&lines, "jobs") else {
        return steps;
    };
    let jobs = children(&lines, jobs);
    for (job, (_, line)) in entries(jobs) {
        let Some((name, _)) = yaml_entry(line) else {
            continue;
        };
        let job_block = children(jobs, job);
        let Some(at) = entries(job_block)
            .find(|(_, (_, line))| yaml_entry(line).is_some_and(|(key, _)| key == "steps"))
            .map(|(at, _)| at)
        else {
            continue;
        };
        let items = children(job_block, at);
        for (item, (indent, line)) in entries(items) {
            let Some(first) = line.strip_prefix('-') else {
                continue;
            };
            let first = first.trim_start();
            let key_indent = indent + (line.len() - first.len());
            let mut keys = Vec::new();
            let mut lines = vec![first.to_string()];
            if let Some((key, value)) = yaml_entry(first) {
                keys.push((key.to_string(), unquote(value).to_string()));
            }
            for &(indent, line) in children(items, item) {
                if indent == key_indent
                    && let Some((key, value)) = yaml_entry(line)
                {
                    keys.push((key.to_string(), unquote(value).to_string()));
                }
                lines.push(line.to_string());
            }
            steps.push(Step {
                job: name.to_string(),
                keys,
                lines,
            });
        }
    }
    steps
}

/// The lines of a YAML file that carry content, as (indent, text), without
/// comments and trailing blanks.
fn yaml_lines(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .filter_map(|line| {
            let line = strip_yaml_comment(line).trim_end();
            let text = line.trim_start();
            (!text.is_empty()).then_some((line.len() - text.len(), text))
        })
        .collect()
}

/// `line` without a `# comment`: a `#` at the start or after a blank, outside
/// a quoted string. A quote only opens a string where a value or a word
/// starts, so an apostrophe in a shell comment does not hide the comment.
fn strip_yaml_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if (c == '"' || c == '\'')
                && (prev.is_whitespace() || "[{,:=(".contains(prev)) =>
            {
                quote = Some(c)
            }
            None if c == '#' && prev.is_whitespace() => return &line[..i],
            None => {}
        }
        prev = c;
    }
    line
}

/// The lines below `lines[at]` that belong to it: those indented further,
/// and for a key that is not itself a list item, a list at its own indent.
fn children<'a>(lines: &'a [(usize, &'a str)], at: usize) -> &'a [(usize, &'a str)] {
    let (indent, line) = lines[at];
    let item = line.starts_with('-');
    let end = lines[at + 1..]
        .iter()
        .position(|&(i, text)| i < indent || (i == indent && (item || !text.starts_with('-'))))
        .map_or(lines.len(), |n| at + 1 + n);
    &lines[at + 1..end]
}

/// The lines of a block at its own level: its keys, or its list items.
fn entries<'a>(
    block: &'a [(usize, &'a str)],
) -> impl Iterator<Item = (usize, (usize, &'a str))> + 'a {
    let indent = block.first().map_or(0, |&(indent, _)| indent);
    block
        .iter()
        .copied()
        .enumerate()
        .filter(move |&(_, (i, _))| i == indent)
}

fn top_level(lines: &[(usize, &str)], key: &str) -> Option<usize> {
    lines
        .iter()
        .position(|&(indent, line)| indent == 0 && yaml_entry(line).is_some_and(|(k, _)| k == key))
}

/// `key: value` on a line, with a leading `- ` and quotes around the key
/// removed. The value is empty when it follows on the lines below.
fn yaml_entry(line: &str) -> Option<(&str, &str)> {
    let line = line.strip_prefix("- ").unwrap_or(line);
    let (key, value) = match line.find(": ") {
        Some(at) => (&line[..at], line[at + 2..].trim()),
        None => (line.strip_suffix(':')?, ""),
    };
    let key = unquote(key.trim());
    (!key.is_empty() && !key.contains(' ')).then_some((key, value))
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    for q in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(q).and_then(|v| v.strip_suffix(q)) {
            return inner;
        }
    }
    value
}

#[test]
fn yaml_lines_drop_comments_but_keep_quoted_hashes() {
    let text = "a: 1 # one\n  # whole line\n\nb: \"x # y\"\nc: it's # gone\nd: a#b\n";
    assert_eq!(
        yaml_lines(text),
        vec![
            (0, "a: 1"),
            (0, "b: \"x # y\""),
            (0, "c: it's"),
            (0, "d: a#b")
        ]
    );
}

#[test]
fn steps_are_read_with_their_job_and_keys() {
    let workflow = r#"
jobs:
  build:
    steps:
      - uses: actions/checkout@v4
      - name: Build
        if: always()
        run: |
          make \
            all
  release:
    runs-on: macos-latest
    steps:
    - name: "Publish"
      uses: 'softprops/action-gh-release@v2'
      with:
        prerelease: true
"#;
    let steps = workflow_steps(workflow);
    let summary: Vec<(&str, Option<&str>, Option<&str>)> = steps
        .iter()
        .map(|step| (step.job.as_str(), step.get("name"), step.get("uses")))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("build", None, Some("actions/checkout@v4")),
            ("build", Some("Build"), None),
            (
                "release",
                Some("Publish"),
                Some("softprops/action-gh-release@v2")
            ),
        ]
    );
    assert_eq!(steps[1].get("if"), Some("always()"));
    assert_eq!(
        steps[1].words(),
        vec![
            "name:", "Build", "if:", "always()", "run:", "|", "make", "all"
        ]
    );
    assert_eq!(steps[2].get("prerelease"), None);
}

/// The smallest workflow that keeps the release rules.
const RELEASE_SAMPLE: &str = r#"
on:
  push:
    tags: ['v*']
jobs:
  sign:
    steps:
      - name: Hand over
        uses: actions/upload-artifact@v4
        with:
          name: signed-release
          path: |
            dist/*.dmg
            dist/*.tar.gz
            dist/*.minisig
            dist/update.json
            dist/SHA256SUMS
  release:
    steps:
      - name: Publish
        uses: softprops/action-gh-release@v2
        with:
          tag_name: ${{ env.RELEASE_TAG }}
          files: |
            dist/*.dmg
            dist/*.tar.gz
            dist/*.minisig
            dist/update.json
            dist/SHA256SUMS
          fail_on_unmatched_files: true
          prerelease: true
          make_latest: false
  channel:
    needs: release
    concurrency:
      group: kanal-vorab
      cancel-in-progress: false
    steps:
      - uses: actions/checkout@v4
      - name: Channel
        run: scripts/update-channel.sh "${GITHUB_REPOSITORY}"
"#;

#[test]
fn release_rules_catch_every_way_around_the_pre_release_flag() {
    assert_eq!(
        release_workflow_problems(RELEASE_SAMPLE),
        Vec::<String>::new()
    );
    let publish = "      - name: Publish\n";
    for (from, to) in [
        ("prerelease: true", "prerelease: false"),
        ("prerelease: true", "# prerelease: true"),
        (
            "prerelease: true",
            "prerelease: ${{ !contains(github.ref, '-') }}",
        ),
        (
            "prerelease: true",
            "prerelease: true\n          make_latest: true",
        ),
        (
            "softprops/action-gh-release@v2",
            "ncipollo/release-action@v1",
        ),
        (
            publish,
            "      - run: gh release edit \"$TAG\" --latest\n      - name: Publish\n",
        ),
        (
            publish,
            "      - run: |\n          gh release \\\n            create x\n      - name: Publish\n",
        ),
        (
            publish,
            "      - name: Publish\n        continue-on-error: true\n",
        ),
        ("    steps:", "    continue-on-error: true\n    steps:"),
        (
            "fail_on_unmatched_files: true",
            "fail_on_unmatched_files: false",
        ),
        ("          make_latest: false\n", ""),
        ("make_latest: false", "make_latest: legacy"),
        ("make_latest: false", "make_latest: \"true\""),
        ("          fail_on_unmatched_files: true\n", ""),
        (
            "            dist/SHA256SUMS\n          fail",
            "          fail",
        ),
        ("            dist/SHA256SUMS\n  release:", "  release:"),
        ("files: |\n            dist/*.dmg\n", "files: |\n"),
        ("path: |\n            dist/*.dmg\n", "path: |\n"),
        ("name: signed-release", "name: signed"),
    ] {
        let mutated = RELEASE_SAMPLE.replacen(from, to, 1);
        assert_ne!(mutated, RELEASE_SAMPLE, "{from} not in the sample");
        assert!(
            !release_workflow_problems(&mutated).is_empty(),
            "not caught: {from:?} -> {to:?}"
        );
    }
}

/// The comparison that keeps `kanal-vorab` moving forward, run as the
/// workflow runs it. The order is semver.org's example plus the versions
/// this project tags; `sort -V` gets every pre-release against its release
/// wrong.
#[test]
fn the_channel_compares_versions_by_semver_precedence() {
    let compare = |a: &str, b: &str| {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join(VERSION_COMPARE);
        let out = std::process::Command::new("python3")
            .arg("-I")
            .arg(&script)
            .args([a, b])
            .output()
            .unwrap_or_else(|err| panic!("cannot run python3 {}: {err}", script.display()));
        let answer = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.code(), answer)
    };
    let ascending = [
        "0.9.3",
        "0.9.4",
        "0.9.10",
        "0.10.0-pre.1",
        "0.10.0-pre.2",
        "0.10.0-pre.10",
        "0.10.0-rc.1",
        "0.10.0",
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
        "2.0.0",
        "10.0.0",
    ];
    let mut cases = vec![
        ("0.9.4", "0.10.0-pre.3", "lower"),
        ("0.10.0", "0.10.0-pre.9", "higher"),
        ("0.10.0-pre.3", "0.10.0-pre.3", "equal"),
        ("1.0.0+build.2", "1.0.0+build.1", "equal"),
        ("1.0.0-rc.1+a", "1.0.0-rc.1", "equal"),
        ("1.0.0-0a", "1.0.0-0", "higher"),
    ];
    for pair in ascending.windows(2) {
        cases.push((pair[0], pair[1], "lower"));
        cases.push((pair[1], pair[0], "higher"));
    }
    for (a, b, expected) in cases {
        assert_eq!(
            compare(a, b),
            (Some(0), expected.to_string()),
            "{a} against {b}"
        );
    }
    for malformed in [
        "v0.9.4",
        "0.9",
        "1.2.3.4",
        "01.2.3",
        "1.2.3-",
        "1.2.3-01",
        "1.2.3-a..b",
        "1.2.3+",
        "",
    ] {
        assert_eq!(
            compare(malformed, "0.9.4"),
            (Some(2), String::new()),
            "{malformed:?}"
        );
        assert_eq!(
            compare("0.9.4", malformed),
            (Some(2), String::new()),
            "{malformed:?}"
        );
    }
}

#[test]
fn branch_filters_are_read_from_the_triggers() {
    let list = |names: &[&str]| Ok(Some(names.iter().map(|n| n.to_string()).collect()));
    let workflow = "on:\n  push:\n    branches: [master, \"rebuild\"] # both\n  pull_request:\n  \
                    schedule:\n    - cron: '0 6 * * 1'\n";
    assert_eq!(
        branch_filter(workflow, "push"),
        list(&["master", "rebuild"])
    );
    assert_eq!(branch_filter(workflow, "pull_request"), Ok(None));
    assert!(branch_filter(workflow, "workflow_dispatch").is_err());

    let block = "\"on\":\n  pull_request:\n    branches:\n    - master\n    # - rebuild\n    \
                 paths: ['src/**']\n  workflow_dispatch:\n";
    assert_eq!(branch_filter(block, "pull_request"), list(&["master"]));
    assert_eq!(
        branch_filter("on:\n  push:\n    tags: ['v*']\n", "push"),
        list(&[])
    );
    assert_eq!(
        branch_filter("on:\n  push:\n    branches: rebuild\n", "push"),
        list(&["rebuild"])
    );
    for unreadable in [
        "on: [push, pull_request]\n",
        "on:\n  push: {branches: [master]}\n",
        "on:\n  push:\n    branches-ignore: [main]\n",
    ] {
        assert!(branch_filter(unreadable, "push").is_err(), "{unreadable}");
    }
}

#[test]
fn release_rules_catch_a_tag_that_skips_the_channel() {
    let channel = "      - name: Channel\n";
    let (head, step) = RELEASE_SAMPLE.split_once(channel).expect("channel step");
    let before_publish = head.replacen(
        "      - name: Publish\n",
        &format!("{channel}{step}      - name: Publish\n"),
        1,
    );
    assert!(!release_workflow_problems(&before_publish).is_empty());

    for (from, to) in [
        (format!("{channel}{step}"), String::new()),
        (
            "scripts/update-channel.sh".into(),
            "scripts/channel.sh".into(),
        ),
        (
            channel.into(),
            format!("{channel}        if: startsWith(github.ref, 'refs/tags/v0.10')\n"),
        ),
        (channel.into(), format!("  late:\n    steps:\n{channel}")),
        ("    needs: release\n".into(), String::new()),
        ("    needs: release\n".into(), "    needs: [sign]\n".into()),
        (
            "    concurrency:\n      group: kanal-vorab\n      cancel-in-progress: false\n".into(),
            String::new(),
        ),
        (
            "group: kanal-vorab".into(),
            "group: ${{ github.ref }}".into(),
        ),
        (
            "cancel-in-progress: false".into(),
            "cancel-in-progress: true".into(),
        ),
        (
            "  channel:\n".into(),
            "  channel:\n    if: ${{ !contains(github.ref_name, '-') }}\n".into(),
        ),
        (channel.into(), format!("{channel}{step}{channel}")),
        ("      - uses: actions/checkout@v4\n".into(), String::new()),
        (
            channel.into(),
            format!(
                "      - run: gh release upload kanal-vorab dist/update.json --clobber\n{channel}"
            ),
        ),
        (
            channel.into(),
            format!("      - run: git tag --list | sort -V | tail -1\n{channel}"),
        ),
    ] {
        let mutated = RELEASE_SAMPLE.replacen(&from, &to, 1);
        assert_ne!(mutated, RELEASE_SAMPLE, "{from} not in the sample");
        assert!(
            !release_workflow_problems(&mutated).is_empty(),
            "not caught: {from:?} -> {to:?}"
        );
    }
}
