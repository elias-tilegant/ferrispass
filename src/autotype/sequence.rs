//! Parse and render KeePass-style auto-type sequences.
//!
//! A sequence is a single string template that mixes literal text with
//! `{TOKEN}` placeholders. The default template
//! `{USERNAME}{TAB}{PASSWORD}{ENTER}` covers ~90% of login forms. v1
//! recognises a strict subset of KeePass's vocabulary — extending later
//! is purely additive (just add a `Token` variant and a `parse_token`
//! arm).
//!
//! Two phases on purpose:
//! 1. `parse` turns the template into `Vec<Token>` once, so the
//!    Settings UI can surface a parse error the moment the user types
//!    a bad sequence — without ever touching enigo / the OS.
//! 2. `render` substitutes the entry's username/password into the
//!    tokens and emits `TypeOp`s. The renderer is the *only* place
//!    that holds the cleartext password as a String — and it returns
//!    `Vec<TypeOp>` rather than executing, so the typer (the only code
//!    that depends on enigo) is fully unit-testable in isolation.
//!
//! A rendered `Vec<TypeOp>` contains the cleartext password inside
//! `TypeOp::SecretText`. Its `Debug` implementation always redacts the value;
//! callers must still drop the operation stream as soon as typing finishes.

use std::fmt;
use std::time::Duration;

/// One unit of the parsed template. Cheap to clone; tokens are scanned
/// once at parse time and reused for every keystroke run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// Type this string verbatim.
    Literal(String),
    /// Replaced with the entry's username at render time.
    Username,
    /// Replaced with the entry's password at render time.
    Password,
    /// Press the Tab key.
    Tab,
    /// Press Return / Enter.
    Return,
    /// Pause for `N` milliseconds. KeePass uses `{DELAY 500}` for sites
    /// that need a moment after Tab before they accept the next field
    /// (some SPAs intercept the focus change).
    Delay(u64),
}

/// One step the typer should execute. Password text stays distinct from
/// ordinary text so the execution layer can apply a last-moment focus guard
/// and avoid exposing it through `Debug` output.
#[derive(Clone, PartialEq, Eq)]
pub enum TypeOp {
    Text(String),
    SecretText(String),
    Tab,
    Return,
    Sleep(Duration),
}

impl fmt::Debug for TypeOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::SecretText(_) => f.write_str("SecretText(<redacted>)"),
            Self::Tab => f.write_str("Tab"),
            Self::Return => f.write_str("Return"),
            Self::Sleep(duration) => f.debug_tuple("Sleep").field(duration).finish(),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    /// `{` without a matching `}`. Almost always a typo — we surface it
    /// rather than silently typing the brace literally, because the
    /// alternative (KeePass behavior: emit the brace as text) hides the
    /// user's mistake from them.
    #[error("unbalanced '{{' — every '{{' must be closed by '}}'")]
    UnbalancedBrace,
    /// `{NOPE}` — token name not in our vocabulary.
    #[error("unknown placeholder: {{{0}}}")]
    UnknownToken(String),
    /// `{DELAY abc}` — the parameter after a space-separated token name
    /// failed to parse as a u64.
    #[error("invalid delay value: {0} (expected milliseconds as an integer)")]
    InvalidDelay(String),
    /// `{DELAY <too-large>}` — caps the parameter so a typo or a
    /// tampered `settings.json` can't park an auto-type task asleep
    /// for hours. Any wait beyond `MAX_DELAY_MS` is almost certainly
    /// unintended.
    #[error("delay too large: {0} ms (max {1} ms)")]
    DelayTooLarge(u64, u64),
}

/// Hard ceiling on `{DELAY n}` (milliseconds). 30 s is well past any
/// realistic "wait for the page to load" need; longer values are a
/// typo or corruption guarantee. The cap runs at parse time so the
/// Settings UI surfaces the error immediately, before the bad
/// template is ever handed to the typer.
pub const MAX_DELAY_MS: u64 = 30_000;

/// The default sequence FerrisPass uses out of the box. Picked because
/// it lines up with the overwhelming majority of HTML and native login
/// forms: focus on the username field, tab moves to password, Return
/// submits. Users who need something custom can edit it in Settings.
pub const DEFAULT_SEQUENCE: &str = "{USERNAME}{TAB}{PASSWORD}{ENTER}";

/// Inputs available to placeholder substitution.
#[derive(Clone)]
pub struct RenderContext {
    pub username: String,
    pub password: String,
}

/// Parse a template into tokens. Empty literal runs are skipped so the
/// renderer doesn't emit zero-length `Text` ops. Brace handling is
/// strict: `{{` is the documented escape for a literal `{`, but
/// production templates almost never need it — we'd rather surface a
/// real typo than swallow it.
pub fn parse(template: &str) -> Result<Vec<Token>, ParseError> {
    let mut out = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                // `{{` → literal `{`. Mirrors how KeePass docs escape
                // a leading brace; without this you can't type a JSON
                // payload as a literal.
                if matches!(chars.peek(), Some('{')) {
                    chars.next();
                    literal.push('{');
                    continue;
                }
                if !literal.is_empty() {
                    out.push(Token::Literal(std::mem::take(&mut literal)));
                }
                let mut body = String::new();
                let mut closed = false;
                for inner in chars.by_ref() {
                    if inner == '}' {
                        closed = true;
                        break;
                    }
                    body.push(inner);
                }
                if !closed {
                    return Err(ParseError::UnbalancedBrace);
                }
                out.push(parse_token(&body)?);
            }
            '}' => {
                // Match the `{{` escape with a `}}` escape so
                // round-tripping is symmetric. A bare `}` is treated as
                // literal — KeePass tolerates this and most users don't
                // know about the escape.
                if matches!(chars.peek(), Some('}')) {
                    chars.next();
                }
                literal.push('}');
            }
            other => literal.push(other),
        }
    }
    if !literal.is_empty() {
        out.push(Token::Literal(literal));
    }
    Ok(out)
}

fn parse_token(body: &str) -> Result<Token, ParseError> {
    // KeePass placeholders are case-insensitive (`{username}` and
    // `{USERNAME}` mean the same thing), so we normalise for matching.
    // Trim too — `{ USERNAME }` is the kind of small slip that should
    // just work. Errors quote the original casing so the user sees
    // exactly what they typed.
    let trimmed = body.trim();
    let upper = trimmed.to_ascii_uppercase();
    // `{DELAY 500}` carries a numeric parameter after a space. Split
    // on the original (cased) string so the error message preserves
    // user input casing.
    if upper.starts_with("DELAY") {
        let original_rest = trimmed[5..].trim();
        if original_rest.is_empty() {
            return Err(ParseError::InvalidDelay(String::new()));
        }
        let ms: u64 = original_rest
            .parse()
            .map_err(|_| ParseError::InvalidDelay(original_rest.to_string()))?;
        if ms > MAX_DELAY_MS {
            return Err(ParseError::DelayTooLarge(ms, MAX_DELAY_MS));
        }
        return Ok(Token::Delay(ms));
    }
    match upper.as_str() {
        "USERNAME" => Ok(Token::Username),
        "PASSWORD" => Ok(Token::Password),
        "TAB" => Ok(Token::Tab),
        "ENTER" | "RETURN" => Ok(Token::Return),
        _ => Err(ParseError::UnknownToken(trimmed.to_string())),
    }
}

/// Materialise a parsed template into the op stream the typer will execute.
/// Adjacent literal and username tokens are batched into `TypeOp::Text`.
/// Password placeholders remain separate `SecretText` operations so each one
/// gets its own last-moment focus check and redacted debug representation.
pub fn render(tokens: &[Token], ctx: &RenderContext) -> Vec<TypeOp> {
    let mut ops = Vec::new();
    let mut pending_text = String::new();

    let flush = |buf: &mut String, ops: &mut Vec<TypeOp>| {
        if !buf.is_empty() {
            ops.push(TypeOp::Text(std::mem::take(buf)));
        }
    };

    for tok in tokens {
        match tok {
            Token::Literal(s) => pending_text.push_str(s),
            Token::Username => pending_text.push_str(&ctx.username),
            Token::Password => {
                flush(&mut pending_text, &mut ops);
                if !ctx.password.is_empty() {
                    ops.push(TypeOp::SecretText(ctx.password.clone()));
                }
            }
            Token::Tab => {
                flush(&mut pending_text, &mut ops);
                ops.push(TypeOp::Tab);
            }
            Token::Return => {
                flush(&mut pending_text, &mut ops);
                ops.push(TypeOp::Return);
            }
            Token::Delay(ms) => {
                flush(&mut pending_text, &mut ops);
                ops.push(TypeOp::Sleep(Duration::from_millis(*ms)));
            }
        }
    }
    flush(&mut pending_text, &mut ops);
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RenderContext {
        RenderContext {
            username: "alice".into(),
            password: "p4ssw0rd".into(),
        }
    }

    #[test]
    fn parses_default_template() {
        let tokens = parse(DEFAULT_SEQUENCE).unwrap();
        assert_eq!(
            tokens,
            vec![Token::Username, Token::Tab, Token::Password, Token::Return],
        );
    }

    #[test]
    fn parses_literal_around_tokens() {
        let tokens = parse("prefix-{USERNAME}-suffix").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Literal("prefix-".into()),
                Token::Username,
                Token::Literal("-suffix".into()),
            ],
        );
    }

    #[test]
    fn placeholder_names_are_case_insensitive() {
        // KeePass is case-insensitive for placeholder names — mirror
        // that so users hand-editing sequences don't get tripped up.
        let tokens = parse("{username}{Tab}{Password}{enter}").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Username, Token::Tab, Token::Password, Token::Return],
        );
    }

    #[test]
    fn whitespace_inside_token_tolerated() {
        // `{ USERNAME }` is a common slip — accept rather than rejecting.
        assert_eq!(parse("{ USERNAME }").unwrap(), vec![Token::Username]);
    }

    #[test]
    fn return_and_enter_aliases() {
        assert_eq!(parse("{RETURN}").unwrap(), vec![Token::Return]);
        assert_eq!(parse("{ENTER}").unwrap(), vec![Token::Return]);
    }

    #[test]
    fn delay_parses_milliseconds() {
        assert_eq!(parse("{DELAY 750}").unwrap(), vec![Token::Delay(750)]);
        // Multiple spaces around the number tolerated.
        assert_eq!(parse("{DELAY  100}").unwrap(), vec![Token::Delay(100)]);
    }

    #[test]
    fn unknown_token_is_rejected() {
        // Errors surface the offending name verbatim so the Settings UI
        // can highlight exactly what's wrong.
        let err = parse("{NOPE}").unwrap_err();
        assert_eq!(err, ParseError::UnknownToken("NOPE".into()));
    }

    #[test]
    fn unbalanced_brace_is_rejected() {
        // Lenient parsing here (typing the `{` as literal) would silently
        // mask user typos — strict surfaces the mistake.
        assert_eq!(
            parse("oops {USERNAME").unwrap_err(),
            ParseError::UnbalancedBrace
        );
    }

    #[test]
    fn invalid_delay_value_is_rejected() {
        let err = parse("{DELAY abc}").unwrap_err();
        assert_eq!(err, ParseError::InvalidDelay("abc".into()));
        // Empty delay is also rejected — `{DELAY}` with no number is
        // almost certainly a typo, not "delay zero ms".
        assert!(matches!(parse("{DELAY}"), Err(ParseError::InvalidDelay(_))));
    }

    #[test]
    fn delay_above_cap_is_rejected() {
        // A typo (`{DELAY 30000000}`) or a tampered settings.json could
        // otherwise park the auto-type task asleep for hours. Surface
        // the cap at parse time so the Settings UI can show it before
        // anything reaches the typer.
        let err = parse(&format!("{{DELAY {}}}", MAX_DELAY_MS + 1)).unwrap_err();
        assert_eq!(
            err,
            ParseError::DelayTooLarge(MAX_DELAY_MS + 1, MAX_DELAY_MS)
        );
        // Boundary: exactly the cap is fine.
        assert_eq!(
            parse(&format!("{{DELAY {}}}", MAX_DELAY_MS)).unwrap(),
            vec![Token::Delay(MAX_DELAY_MS)],
        );
    }

    #[test]
    fn double_brace_escapes_to_literal() {
        // `{{` → `{`, `}}` → `}`. Lets users type templated JSON or
        // CLI flag syntax without the parser hijacking braces.
        let tokens = parse("{{user}} = {USERNAME}").unwrap();
        assert_eq!(
            tokens,
            vec![Token::Literal("{user} = ".into()), Token::Username,],
        );
    }

    #[test]
    fn render_default_emits_username_tab_password_return() {
        let tokens = parse(DEFAULT_SEQUENCE).unwrap();
        let ops = render(&tokens, &ctx());
        assert_eq!(
            ops,
            vec![
                TypeOp::Text("alice".into()),
                TypeOp::Tab,
                TypeOp::SecretText("p4ssw0rd".into()),
                TypeOp::Return,
            ],
        );
    }

    #[test]
    fn render_merges_adjacent_text_into_single_op() {
        // `prefix{USERNAME}@host{PASSWORD}` -> one fast_text call per
        // pre-Tab segment. This matters: per-character key events on
        // macOS are >10× slower than a single CGEventKeyboardSetUnicodeString.
        let tokens = parse("prefix-{USERNAME}@host").unwrap();
        let ops = render(&tokens, &ctx());
        assert_eq!(ops, vec![TypeOp::Text("prefix-alice@host".into())]);
    }

    #[test]
    fn render_delay_becomes_sleep_op() {
        let tokens = parse("{USERNAME}{DELAY 250}{PASSWORD}").unwrap();
        let ops = render(&tokens, &ctx());
        assert_eq!(
            ops,
            vec![
                TypeOp::Text("alice".into()),
                TypeOp::Sleep(Duration::from_millis(250)),
                TypeOp::SecretText("p4ssw0rd".into()),
            ],
        );
    }

    #[test]
    fn every_password_placeholder_is_a_separate_secret_operation() {
        let tokens = parse("{PASSWORD}{TAB}{PASSWORD}").unwrap();
        assert_eq!(
            render(&tokens, &ctx()),
            vec![
                TypeOp::SecretText("p4ssw0rd".into()),
                TypeOp::Tab,
                TypeOp::SecretText("p4ssw0rd".into()),
            ],
        );
    }

    #[test]
    fn secret_operation_debug_output_is_redacted() {
        let rendered = format!("{:?}", TypeOp::SecretText("p4ssw0rd".into()));
        assert_eq!(rendered, "SecretText(<redacted>)");
        assert!(!rendered.contains("p4ssw0rd"));
    }
}
