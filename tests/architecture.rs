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
//! - a lint level weaker than `deny`, and code the scan cannot see
//!   (`include!`, a `#[path]` that leaves the directory).
//!
//! The scan works on tokens, not lines. Comments, doc comments, string and
//! character literals are skipped, so a word in a message or a comment is not
//! a finding. What it cannot see is unsafe code that another crate's macro
//! generates without the word `unsafe` in this crate's source. rustc does not
//! report `unsafe_code` for code expanded from another crate's macro either;
//! those crates are reviewed with the supply chain (docs/release-process.md).

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

/// Every target the `[lints]` table of `Cargo.toml` applies to.
const SOURCE_DIRS: &[&str] = &["src", "tests", "examples", "benches"];
const SOURCE_FILES: &[&str] = &["build.rs"];

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
fn the_lint_level_still_denies_unsafe_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    let level = lint_level(&manifest, "unsafe_code");
    assert!(
        matches!(level.as_deref(), Some("deny" | "forbid")),
        "Cargo.toml sets unsafe_code to {level:?} under [lints.rust]; the exception list \
         relies on \"deny\""
    );
    for name in [".cargo/config.toml", ".cargo/config"] {
        if let Ok(config) = fs::read_to_string(root.join(name)) {
            for word in ["unsafe_code", "cap-lints"] {
                assert!(
                    !config.contains(word),
                    "{name} mentions {word}; lint levels belong in Cargo.toml"
                );
            }
        }
    }
}

// ---------------------------------------------------------------- scanner --

fn rust_sources(root: &Path) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    for dir in SOURCE_DIRS {
        let dir = root.join(dir);
        if dir.is_dir() {
            collect_rust_files(&dir, &mut files);
        }
    }
    for file in SOURCE_FILES {
        let file = root.join(file);
        if file.is_file() {
            files.push(file);
        }
    }
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

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|err| panic!("cannot list {}: {err}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The value of `name` in the `[lints.rust]` table, if it is a plain level
/// or a table with a `level` key.
fn lint_level(manifest: &str, name: &str) -> Option<String> {
    let mut in_table = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_table = line == "[lints.rust]";
            continue;
        }
        if !in_table || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != name {
            continue;
        }
        return ["forbid", "deny", "warn", "allow"]
            .into_iter()
            .find(|level| value.contains(&format!("\"{level}\"")))
            .map(str::to_string);
    }
    None
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
}

fn parse(source: &str) -> Result<Parsed, String> {
    let tokens = tokenize(source)?;
    let mut partner = vec![None; tokens.len()];
    let mut enclosing_brace = vec![None; tokens.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        enclosing_brace[i] = stack
            .iter()
            .rev()
            .find(|&&open| tokens[open].is_punct('{'))
            .copied();
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
const UNSAFE_ATTRIBUTES: &[&str] = &["no_mangle", "export_name", "link_section"];

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
        if token.name() != Some("unsafe_code") {
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
        let next_is_bang = t.get(i + 1).is_some_and(|t| t.is_punct('!'));
        let site = token.is_keyword("unsafe")
            || (token.is_keyword("global_asm") && next_is_bang)
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
        if token.is_keyword("include") && next_is_bang {
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
            && (target.starts_with('/') || target.split('/').any(|part| part == ".."))
        {
            problems.push((
                name.line,
                format!("#[path = \"{target}\"] leaves the scanned directory"),
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
    "#;
    assert_eq!(
        unsafe_lines(source),
        vec![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    );
    assert_eq!(
        problem_lines(source),
        vec![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    );
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
    "#;
    assert_eq!(problem_lines(source), vec![2, 3, 4]);
}

#[test]
fn malformed_source_is_an_error_not_a_pass() {
    assert!(scan("fn a() { \"open string }").is_err());
    assert!(scan("fn a() { /* open comment }").is_err());
    assert!(scan("fn a() { (] }").is_err());
}

#[test]
fn lint_levels_are_read_from_the_lints_table() {
    let manifest = "[package]\nunsafe_code = \"allow\"\n[lints.rust]\nunsafe_code = \"deny\"\n";
    assert_eq!(lint_level(manifest, "unsafe_code").as_deref(), Some("deny"));
    let table = "[lints.rust]\nunsafe_code = { level = \"warn\", priority = 1 }\n";
    assert_eq!(lint_level(table, "unsafe_code").as_deref(), Some("warn"));
    assert_eq!(lint_level("[lints.rust]\n", "unsafe_code"), None);
}
