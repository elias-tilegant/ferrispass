//! The quick access panel while the app is locked.
//!
//! What you type before the vault is unlocked goes into a [`TypedAhead`]
//! buffer, and the panel shows only how many characters there are, as dots.
//! The rules, which the tests in `locked/tests.rs` hold:
//!
//! 1. Secure event input is on as soon as the locked panel is the key
//!    window, before the first key, and never earlier: the buffer begins in
//!    the window's activation observer, on the change to active and only
//!    when `is_window_active` says so, never when the panel opens, although
//!    it opens with `focus: true` while another app is still in front. When
//!    a buffer is wiped while the panel stays key and locked, the next one
//!    begins at once, so the key panel never takes keys without secure
//!    input. When macOS refuses secure input, there is no buffer, and the
//!    panel sends you to the main window.
//! 2. Every tick and every key wipes the buffer once 30 seconds have passed
//!    since its first key. It is also wiped when the panel closes, when you
//!    click into another app, and on Escape.
//! 3. When the panel loses key status while its own Touch ID prompt is up,
//!    from Return or the button until macOS answers, the dialog has taken
//!    it: the panel stays open and lets go of secure input, and the buffer
//!    keeps its text and its clock. Losing key status any other way closes
//!    the panel and wipes the buffer, also while the vault still opens after
//!    the answer.
//! 4. The field is a plain element with the role of a text input and the
//!    name "Search". It never has a value: the dots are drawn as text that
//!    assistive technology does not read, and nothing typed reaches a node,
//!    a log line or a `Debug` output.
//! 5. While locked, the panel shows nothing from a vault: no favorite, no
//!    recently used entry, no row and no title. A missing vault file is
//!    named by its file name, cleaned up for display. Every text comes from
//!    `quick.locked.*`.
//! 6. Touch ID starts from the panel only in [`PanelView::on_return`] and
//!    in the unlock button's handler, each through `touch_id::confirm` and
//!    `touch_id::unlock_from_panel`. Opening, typing, becoming key again, a
//!    tick, an expired buffer and a prompt macOS withdrew start nothing, and
//!    neither does Return or the button while an unlock is under way.
//!    `TOUCH_ID_CALL_SITES` in `tests/architecture.rs` lists the calls.
//! 7. As soon as macOS has answered, the panel makes itself the key window
//!    again, focuses its field and takes secure input back, before the
//!    vault is opened and before any Return counts. Should the panel be key
//!    again before that, it takes secure input back then. The 30 seconds
//!    run on through the dialog.
//! 8. What was typed becomes the search only after the panel's own unlock
//!    opened the vault, and only through `take_query`: text that expired
//!    meanwhile gives an empty search, and several words without a single
//!    match are dropped, since they look like a password typed into the
//!    wrong window. The buffer is searched only there, so the rule runs
//!    there, on the text `take_query` gave. A vault opened any other way
//!    gets nothing of what was typed: the buffer is wiped as it opens, and
//!    secure input goes with it.

use std::path::Path;
use std::time::{Duration, SystemTime};

use gpui::{
    AnyElement, App, Context, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Task, Text, Window, div,
};

use super::view::PanelView;
use crate::app::unlock::locked_panel::{self, Kind};
use crate::app::unlock::touch_id::{self, Answer, Confirmed, OnAnswered, OnDone, PanelTrigger};
use crate::app::unlock::typed_ahead::{self, TypedAhead};
use crate::app::{AppState, VaultStatus};
use crate::tr;
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::components::{Button, FooterTone, PanelFooter};
use crate::ui::ds::tokens::dimension;
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};
use crate::ui::vm::panel::FooterModel;

/// How often the locked panel looks at the clock.
pub const TICK: Duration = Duration::from_secs(1);

/// The most characters of a file name the panel shows.
const NAME_MAX: usize = 60;

/// What the locked panel holds. It lives with the window: the buffer is not
/// `Send`, and closing the window wipes it.
#[derive(Default)]
pub(super) struct Locked {
    buffer: Option<TypedAhead>,
    kind: Option<Kind>,
    /// The generation of the panel's own unlock attempt, while one runs.
    attempt: Option<u64>,
    /// The panel's own Touch ID prompt is up: from the start of the attempt
    /// until macOS answers. Losing key status then is the dialog taking it.
    prompt_up: bool,
    dots: usize,
    /// macOS refused secure event input for a buffer: this window takes no
    /// keys and sends you to the main window.
    refused: bool,
}

impl Locked {
    /// The case the panel shows, read from `state`.
    fn read_kind(&mut self, state: &AppState, allow_device_passcode: bool) {
        let kind = locked_panel::kind(state, allow_device_passcode);
        self.kind = Some(match kind {
            Kind::TouchId { .. } if self.refused => Kind::NeedsMainWindow,
            kind => kind,
        });
    }

    fn kind(&self) -> Kind {
        self.kind.clone().unwrap_or(Kind::NeedsMainWindow)
    }

    /// Overwrites what was typed and lets go of secure input.
    pub(super) fn wipe(&mut self) {
        if let Some(mut buffer) = self.buffer.take() {
            buffer.wipe();
        }
        self.dots = 0;
    }

    /// Wipes the buffer if its 30 seconds are up at `now`; says whether it
    /// did.
    fn expire(&mut self, now: SystemTime) -> bool {
        let expired = self
            .buffer
            .as_mut()
            .is_some_and(|buffer| buffer.expire_if_due(now));
        if expired {
            self.wipe();
        }
        expired
    }

    fn has_text(&self) -> bool {
        self.buffer
            .as_ref()
            .is_some_and(|buffer| !buffer.is_empty())
    }

    /// The Touch ID dialog has key status: secure input goes, the text and
    /// its clock stay.
    fn release(&mut self) {
        if let Some(buffer) = &mut self.buffer {
            buffer.release_secure_input();
        }
    }

    /// The panel is key again: a buffer that let go of secure input takes it
    /// back, with its text and its clock; one that holds it is left as it
    /// is. If macOS refuses, the text is wiped.
    fn rearm(&mut self, cx: &App) {
        if let Some(buffer) = &mut self.buffer
            && buffer.rearm_secure_input(cx).is_err()
        {
            self.wipe();
        }
    }
}

/// The wall clock the 30 seconds run on, as the buffer's own clock does.
/// Tests move it forward.
fn now() -> SystemTime {
    #[cfg(test)]
    {
        SystemTime::now() + tests::wall_clock_ahead()
    }
    #[cfg(not(test))]
    {
        SystemTime::now()
    }
}

/// Makes the panel the key window again, after the Touch ID dialog took
/// that from it. It brings the panel forward, not the app: the panel is a
/// non-activating window. The one place the panel asks for key status.
fn take_key(window: &Window) {
    #[cfg(test)]
    tests::note_key_taken();
    window.activate_window();
}

/// Whether the settings let macOS ask for the Mac password when no Touch ID
/// sensor is in reach.
fn allow_device_passcode(cx: &mut App) -> bool {
    crate::app::with_shared_shell(cx, |shell, _| {
        shell.settings().biometric_allow_passcode_fallback
    })
    .unwrap_or_else(|| crate::app::AppSettings::default().biometric_allow_passcode_fallback)
}

/// `text` as the panel may show a name it read from the disk: without
/// control characters and the characters that turn the writing direction,
/// and cut after [`NAME_MAX`] characters.
pub(super) fn display_untrusted(text: &str) -> SharedString {
    let mut shown: String = text
        .chars()
        .filter(|c| !c.is_control() && !turns_direction(*c))
        .collect();
    if shown.chars().count() > NAME_MAX {
        shown = shown.chars().take(NAME_MAX).collect();
        shown.push('\u{2026}');
    }
    shown.into()
}

/// The marks that turn the writing direction around, so that a name could
/// read differently from what it is.
fn turns_direction(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The vault file's name, as the panel shows it.
pub(super) fn file_name(path: &Path) -> SharedString {
    display_untrusted(
        &path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

/// The vault's name without `.kdbx`, as the panel shows it.
fn vault_name(path: &Path) -> SharedString {
    display_untrusted(
        &path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

/// What the locked panel shows: the case, the number of dots, and the vault
/// the case is about.
#[derive(Clone)]
pub(super) struct Look {
    pub(super) kind: Kind,
    pub(super) dots: usize,
    pub(super) vault: SharedString,
}

/// Every element the locked panel draws. Built in one place, so the tests
/// read what the window shows.
pub(super) struct Built {
    pub(super) field: Stateful<gpui::Div>,
    pub(super) sentence: Text,
    pub(super) button: Option<Button>,
    pub(super) footer: PanelFooter,
}

/// The sentence for `kind`.
pub(super) fn sentence(kind: &Kind, vault: &SharedString) -> SharedString {
    match kind {
        Kind::TouchId { .. } => tr!("quick.locked.typed_ahead"),
        Kind::NeedsMainWindow => tr!("quick.locked.needs_main_window"),
        Kind::NotSetUp => tr!("quick.locked.not_set_up"),
        Kind::FileMissing(_) => tr!("quick.locked.file_missing", vault = vault),
        Kind::Unlocking => tr!("quick.locked.still_unlocking", vault = vault),
    }
}

/// The name of the button for `kind`: unlocking for Touch ID, with the Mac
/// password when no sensor is in reach, and setting up when there is
/// nothing yet. No button otherwise.
pub(super) fn button_name(kind: &Kind) -> Option<SharedString> {
    match kind {
        Kind::TouchId {
            device_password: false,
        } => Some(tr!("quick.locked.unlock_touch_id")),
        Kind::TouchId {
            device_password: true,
        } => Some(tr!("quick.locked.unlock_mac_password")),
        Kind::NotSetUp => Some(tr!("quick.locked.set_up")),
        Kind::NeedsMainWindow | Kind::FileMissing(_) | Kind::Unlocking => None,
    }
}

/// The button for `kind`, named by [`button_name`]. Return presses the
/// unlock button.
fn button(kind: &Kind) -> Option<Button> {
    let name = button_name(kind)?;
    Some(match kind {
        Kind::NotSetUp => Button::secondary("panel-set-up", name),
        _ => Button::primary("panel-unlock", name).default_button(),
    })
}

/// The footer: Cmd+O where only the main window helps, and "Search saved"
/// once something is typed.
fn footer_model(look: &Look) -> FooterModel {
    let main = match look.kind {
        Kind::NeedsMainWindow | Kind::FileMissing(_) => Some(("cmd-o", tr!("action.open.label"))),
        Kind::TouchId { .. } | Kind::NotSetUp | Kind::Unlocking => None,
    };
    FooterModel {
        primary: None,
        main,
        tone: FooterTone::Normal,
        more: Vec::new(),
        note: (look.dots > 0).then(|| tr!("quick.locked.search_pending")),
    }
}

/// The dots for `count` characters.
fn dots(count: usize) -> SharedString {
    "\u{2022}".repeat(count).into()
}

/// Builds every element of the locked panel for `look`. The field tracks
/// `focus`; the caller adds the handlers.
pub(super) fn build(look: &Look, focus: &FocusHandle) -> Built {
    let field = div()
        .id("panel-locked-field")
        .track_focus(focus)
        .role(Role::TextInput)
        .aria_label(tr!("a11y.search"))
        .child(Text::new_inaccessible(dots(look.dots)));
    Built {
        field,
        sentence: Text::new(
            ElementId::Name("panel-locked-sentence".into()),
            sentence(&look.kind, &look.vault),
        ),
        button: button(&look.kind),
        footer: super::view::footer(&footer_model(look)),
    }
}

impl PanelView {
    /// Whether no vault is open, which is when the panel is locked.
    pub(super) fn is_locked(&self, cx: &App) -> bool {
        !matches!(self.state.read(cx).vault_status(), VaultStatus::Open { .. })
    }

    /// What the locked panel shows now.
    pub(super) fn look(&self, cx: &App) -> Look {
        let kind = self.locked.kind();
        let state = self.state.read(cx);
        let vault = match &kind {
            Kind::FileMissing(path) => file_name(path),
            _ => state
                .current_vault_path()
                .or_else(|| state.recents().first().map(|recent| recent.path.clone()))
                .map(|path| vault_name(&path))
                .unwrap_or_default(),
        };
        Look {
            kind,
            dots: self.locked.dots,
            vault,
        }
    }

    /// Reads the case again from the state.
    pub(super) fn read_kind(&mut self, cx: &mut Context<Self>) {
        let allow = allow_device_passcode(cx);
        self.locked.read_kind(self.state.read(cx), allow);
    }

    /// The window's activation changed. Key: the buffer takes secure input
    /// back, or a buffer begins. Not key while the panel's own prompt is up:
    /// the Touch ID dialog has the keyboard, so secure input goes and the
    /// panel stays. Not key otherwise, a click into another app: the panel
    /// closes and the buffer is wiped.
    pub(super) fn on_activation(
        &mut self,
        active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if active {
            self.locked.rearm(cx);
            self.ensure_buffer(window, cx);
            return;
        }
        if self.is_locked(cx) && self.locked.prompt_up {
            self.locked.release();
            cx.notify();
            return;
        }
        self.close(window, cx);
    }

    /// Begins a buffer when the panel is the key window, locked, offers
    /// Touch ID or is unlocking, and has none. Secure input is on from here,
    /// before any key. Unlocking counts, so that keys typed while the vault
    /// opens after Touch ID are shielded too, also when the buffer before
    /// was wiped meanwhile. If macOS refuses secure input, there is no
    /// buffer and the panel sends you to the main window.
    pub(super) fn ensure_buffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !window.is_window_active() || !self.is_locked(cx) || self.locked.refused {
            return;
        }
        self.read_kind(cx);
        let collects = matches!(
            self.locked.kind,
            Some(Kind::TouchId { .. } | Kind::Unlocking)
        );
        if self.locked.buffer.is_some() || !collects {
            return;
        }
        match TypedAhead::begin(cx) {
            Ok(buffer) => self.locked.buffer = Some(buffer),
            Err(_) => {
                self.locked.refused = true;
                self.locked.kind = Some(Kind::NeedsMainWindow);
            }
        }
        cx.notify();
    }

    /// A key in the locked field: a character goes into the buffer, Backspace
    /// takes the last one. Return and Escape reach the panel as actions. Keys
    /// with Command, Control or Fn are left to the bindings.
    fn on_locked_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.locked.expire(now()) {
            self.ensure_buffer(window, cx);
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.function {
            return;
        }
        if let Some(buffer) = &mut self.locked.buffer {
            if keystroke.key == "backspace" {
                buffer.backspace();
            } else if let Some(typed) = &keystroke.key_char {
                for c in typed.chars().filter(|c| !c.is_control()) {
                    buffer.push(c);
                }
            }
            self.locked.dots = buffer.len();
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Whether Return and the unlock button may start Touch ID now: the
    /// panel offers it and no unlock is under way.
    fn offers_touch_id(&mut self, cx: &mut Context<Self>) -> bool {
        self.read_kind(cx);
        matches!(self.locked.kind, Some(Kind::TouchId { .. }))
    }

    /// Return in the locked panel: one of the two ways to Touch ID.
    pub(super) fn on_return(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.offers_touch_id(cx) {
            return;
        }
        if let Some(token) = touch_id::confirm(PanelTrigger::ReturnKey) {
            self.start_unlock(token, window, cx);
        }
    }

    /// A click on the unlock button: the other way to Touch ID.
    fn on_unlock_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.offers_touch_id(cx) {
            return;
        }
        if let Some(token) = touch_id::confirm(PanelTrigger::UnlockButtonClick) {
            self.start_unlock(token, window, cx);
        }
    }

    /// Hands the confirmation to the shared Touch ID flow. When it starts
    /// nothing (a cost question, the header not read, an unlock under way),
    /// the case says why. The field keeps the keyboard, so a click on the
    /// button leaves the next key in the buffer.
    fn start_unlock(&mut self, token: Confirmed, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle();
        let answered = cx.entity().downgrade();
        let done = answered.clone();
        let on_answered: OnAnswered = Box::new(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = answered.update(cx, |view, cx| view.on_answered(window, cx));
            });
        });
        let on_done: OnDone = Box::new(move |answer, cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = done.update(cx, |view, cx| view.on_done(answer, window, cx));
            });
        });
        let allow = allow_device_passcode(cx);
        self.locked.attempt =
            touch_id::unlock_from_panel(token, self.state.clone(), allow, on_answered, on_done, cx);
        self.locked.prompt_up = self.locked.attempt.is_some();
        window.focus(&self.focus, cx);
        self.read_kind(cx);
        cx.notify();
    }

    /// macOS has answered, and the vault is not opened yet. The Touch ID
    /// dialog took key status and macOS does not give it back: the panel
    /// takes it, focuses its field and takes secure input again, so keys
    /// typed while the vault opens stay here and shielded. A buffer that
    /// cannot take it again is wiped, and the next one begins as soon as the
    /// panel is key, while the vault still opens.
    fn on_answered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.locked.prompt_up = false;
        take_key(window);
        window.focus(&self.focus, cx);
        self.locked.rearm(cx);
        self.ensure_buffer(window, cx);
        cx.notify();
    }

    /// The answer is applied. With the vault open, what was typed becomes
    /// the search, unless it expired or looks like a password. After a
    /// failed or withdrawn prompt, or a vault that did not open, the buffer
    /// stays with its clock and the panel offers what the state allows; the
    /// next attempt needs Return or the button again. A stale answer
    /// changes nothing.
    fn on_done(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        self.locked.attempt = None;
        self.locked.prompt_up = false;
        match answer {
            Answer::Opened(_) => self.hand_over(window, cx),
            Answer::NotOpened | Answer::Failed(_) | Answer::Interrupted => {
                self.read_kind(cx);
                self.ensure_buffer(window, cx);
            }
            Answer::Stale => {}
        }
        cx.notify();
    }

    /// The vault is open: the buffer gives its text, and with it goes secure
    /// input. Several words without a single match are dropped there and
    /// then; anything else becomes the panel's search.
    fn hand_over(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(buffer) = self.locked.buffer.take() else {
            return;
        };
        self.locked.dots = 0;
        let query = buffer.take_query(now());
        let hits = self
            .state
            .read(cx)
            .ranked_for_panel(&query, crate::ui::vm::panel::MAX_HITS)
            .len();
        if typed_ahead::is_password_like(&query, hits) {
            drop(query);
            return;
        }
        let state = self.state.clone();
        self.search
            .update(cx, |input, cx| input.set_value(query.as_str(), window, cx));
        self.vm.update(cx, |vm, cx| {
            vm.set_query(query.as_str(), state.read(cx));
            vm.note_unlocked_for_query();
            cx.notify();
        });
    }

    /// Escape in the locked panel: what was typed is wiped first, and the
    /// next buffer begins at once; with nothing typed, the panel closes.
    /// Says whether Escape is done.
    pub(super) fn escape_locked(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.locked.has_text() {
            return false;
        }
        self.locked.wipe();
        self.ensure_buffer(window, cx);
        cx.notify();
        true
    }

    /// The state changed while the panel is open. A vault opened some other
    /// way than the panel's own unlock takes nothing of what was typed: the
    /// buffer is wiped, and secure input goes with it.
    pub(super) fn locked_state_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_locked(cx) {
            self.locked.wipe();
            return;
        }
        self.read_kind(cx);
        self.ensure_buffer(window, cx);
        cx.notify();
    }

    /// One tick: the buffer expires 30 seconds after its first key, also
    /// while the Touch ID dialog is up, and a panel still key and locked
    /// begins the next one. Says whether the panel is still locked.
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.is_locked(cx) {
            return false;
        }
        if self.locked.expire(now()) {
            self.ensure_buffer(window, cx);
            cx.notify();
        }
        true
    }

    /// Ticks every [`TICK`] for as long as the panel is locked.
    pub(super) fn start_tick(window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let locked = view
                    .update_in(cx, |view, window, cx| view.tick(window, cx))
                    .unwrap_or(false);
                if !locked {
                    break;
                }
            }
        })
    }

    /// The locked panel: the field with its dots, the sentence for the case,
    /// the button, and the footer.
    pub(super) fn render_locked(&mut self, cx: &mut Context<Self>) -> (AnyElement, PanelFooter) {
        let look = self.look(cx);
        let text = cx.ds().colors.text_primary.hsla;
        let secondary = cx.ds().colors.text_secondary.hsla;
        let Built {
            field,
            sentence,
            button,
            footer,
        } = build(&look, &self.focus);
        let field = field
            .on_key_down(cx.listener(Self::on_locked_key_down))
            .text_style(TextStyle::Search)
            .text_color(text)
            .min_h(u(dimension::SPACE_24));
        let button = button.map(|button| match look.kind {
            Kind::NotSetUp => button.on_click(cx.listener(|view, _, window, cx| {
                view.show_in_app(window, cx);
            })),
            _ => button.on_click(cx.listener(|view, _, window, cx| {
                view.on_unlock_click(window, cx);
            })),
        });
        let body = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_8))
            .child(field)
            .child(
                div()
                    .text_style(TextStyle::Body)
                    .text_color(secondary)
                    .child(sentence),
            )
            .children(button.map(|button| div().child(button)))
            .into_any_element();
        (body, footer)
    }
}

/// What the locked panel holds, for the tests.
#[cfg(test)]
impl Locked {
    pub(super) fn dots_for_test(&self) -> usize {
        self.dots
    }

    pub(super) fn has_buffer(&self) -> bool {
        self.buffer.is_some()
    }

    pub(super) fn kind_for_test(&self) -> Kind {
        self.kind()
    }

    pub(super) fn attempt_for_test(&self) -> Option<u64> {
        self.attempt
    }
}

#[cfg(test)]
pub(crate) mod tests;
