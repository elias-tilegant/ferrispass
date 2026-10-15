# GPUI-Fähigkeitsmatrix

Welche Fähigkeiten von GPUI der UI/UX-Neubau braucht, wie sie in der alten Zed-Revision und im neuen Snapshot aussehen, und was die Spikes dazu herausgefunden haben. Grundlage ist Plan 20, 4.7 und Kapitel 5. Angelegt mit P0.02 am 15.10.2026 auf `p0-01-gpui-kit`.

- **Zed 1870e26** ist der Stand, den `master` bis v0.9.2 per Git-Abhängigkeit gebaut hat.
- **gpui-pre 0.3.8** ist der crates.io-Snapshot von zed@279fe07, den FerrisPass seit P0.01 nutzt (`Cargo.toml`, Abschnitt GPUI).

**Belege.** Pfade in der Spalte "Beleg" beginnen unter `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`. Pfade in der Spalte "Zed 1870e26" beginnen unter `~/.cargo/git/checkouts/zed-a70e2ad075855582/1870e26/crates/`. Steht nur `:Zeile`, gilt die zuletzt genannte Datei derselben Zelle. Alle Zeilen sind am 15.10.2026 gegen die Quellen gelesen. Bei jeder Anhebung von gpui-pre verschieben sich die Zeilen, dann wird die Matrix neu belegt (Plan 20, 4.6).

**Laufversuch.** Alles, was ein Fenster oder die echte AppKit-Plattform braucht, läuft nicht nebenbei auf Elias' Arbeits-Mac, sondern im Spike, auf dessen Zweig `spike/p0-NN-name`. Solche Zeilen stehen auf "ausstehend, Spike-Lauf". Der Spike trägt das Ergebnis in die Zeile und in den Abschnitt Spike-Ergebnisse ein.

## Matrix

| Fähigkeit | Zed 1870e26 | gpui-pre 0.3.8 | Beleg | Laufversuch | Spike |
|---|---|---|---|---|---|
| `WindowKind::PopUp`: Fensterklasse | NSPanel-Unterklasse `GPUIPanel` mit Stil `NonactivatingPanel` (`gpui_macos/src/window.rs:134`, `:945-947`) | gleich | `gpui-pre-macos-0.3.8/src/window.rs:129` (`GPUIPanel` aus `NSPanel`), `:95-96` (Maske `1 << 7`), `:1019-1022` (PopUp: Maske plus `PANEL_CLASS`) | ausstehend, Spike-Lauf | SP03, SP05 |
| `WindowKind::PopUp`: Ebene | `NSPopUpWindowLevel` = 101 (`gpui_macos/src/window.rs:94`, `:1159`) | gleich | `gpui-pre-macos-0.3.8/src/window.rs:103`, `:1231` | ausstehend, Spike-Lauf | SP03 |
| `WindowKind::PopUp`: Spaces | `CanJoinAllSpaces` und `FullScreenAuxiliary` (`gpui_macos/src/window.rs:1164-1167`) | gleich | `gpui-pre-macos-0.3.8/src/window.rs:1236-1239` | ausstehend, Spike-Lauf | SP05, SP03 |
| `WindowKind::PopUp`: Maus ohne Fokus | Tracking-Area mit `ActiveAlways`, im PopUp-Zweig selbst angelegt (`gpui_macos/src/window.rs:1145-1157`) | gleich, nur in die Hilfsfunktion `add_mouse_tracking_area` verschoben, dieselben vier Optionen | `gpui-pre-macos-0.3.8/src/window.rs:1229`, `:2488-2508` | ausstehend, Spike-Lauf | SP03 |
| `WindowKind::AnchoredPopup` | auf macOS abgelehnt (`gpui_macos/src/platform.rs:659`) | gleich | `gpui-pre-macos-0.3.8/src/platform.rs:772` | nicht nötig, Quelltext eindeutig | keiner |
| Key-Status: `canBecomeKeyWindow` | fest `YES` für `GPUIWindow` und `GPUIPanel`, beide aus `build_window_class` (`gpui_macos/src/window.rs:375-378`, `:133-134`) | **unverändert fest `YES`.** Ein PopUp kann Key werden, sobald es fokussiert wird. SP03 Variante b (eigene Unterklasse) bleibt nötig | `gpui-pre-macos-0.3.8/src/window.rs:438-441` (`canBecomeKeyWindow`), `:434-437` (`canBecomeMainWindow`), `:128-129` (beide Klassen) | ausstehend, Spike-Lauf | SP03 |
| Key-Status: `focus: false` mit `orderFront` | `focus && show` ruft `makeKeyAndOrderFront`, sonst nur `orderFront` (`gpui_macos/src/window.rs:1216-1220`) | gleich. `PlatformWindow::activate` macht das Fenster dagegen immer Key | `gpui-pre-macos-0.3.8/src/window.rs:1288-1292`, `activate` `:1788`, `:1797` | ausstehend, Spike-Lauf | SP03, SP01 |
| `acceptsFirstMouse` | registriert, liefert `YES` (`gpui_macos/src/window.rs:288`, `:3195`) | gleich, merkt sich zusätzlich `first_mouse` | `gpui-pre-macos-0.3.8/src/window.rs:282-285`, `:3521-3526` | ausstehend, Spike-Lauf | SP03 (c) |
| `window.prompt` | `NSAlert` als Sheet am Fenster (`gpui_macos/src/window.rs:1510`, `:1531`, `:1586`; `gpui/src/window.rs:6244`) | gleich. GPUI Kit 0.7.1 setzt keinen eigenen `PromptBuilder` (kein Treffer für `set_prompt_builder` in `gpui-component-0.7.1` und `gpui-base-0.7.1`), es bleibt also beim nativen Dialog | `gpui-pre-macos-0.3.8/src/window.rs:1704`, `:1725` (`NSAlert::new`), `:1780` (`beginSheetModalForWindow`); `gpui-pre-0.3.8/src/window.rs:6529`, `:6550-6553` (`PromptBuilder::Default` geht zur Plattform) | ausstehend, Spike-Lauf | keiner |
| Dock-Menü: `set_dock_menu` | vorhanden (`gpui/src/app.rs:2435`; `gpui_macos/src/platform.rs:1061`, Delegat `applicationDockMenu` `:139`) | gleich | `gpui-pre-0.3.8/src/app.rs:2679-2681`; `gpui-pre-macos-0.3.8/src/platform.rs:1172-1182`, Delegat `:147-150`, Handler `:1644` | ausstehend, Spike-Lauf | keiner |
| `on_open_urls`: Zustellung | Callback am `Application`-Builder, Delegat `application:openURLs:` (`gpui/src/app.rs:270`; `gpui_macos/src/platform.rs:143`, `:777`, `:1421-1443`) | gleich. **Ohne registrierten Callback verwirft der Delegat die URLs** (`take()` ohne Puffer). Für den Kaltstart muss `on_open_urls` also vor `run` gesetzt sein | `gpui-pre-0.3.8/src/app.rs:287-293`; `gpui-pre-macos-0.3.8/src/platform.rs:151-154`, `:891-893`, `:1568`, `:1585` | ausstehend, Spike-Lauf | SP06 |
| `on_app_quit` | vorhanden, nicht abbrechbar, Handler bekommen 200 ms (`gpui/src/app.rs:2352`, `:78`; `gpui/src/app/context.rs:208`; Delegat `applicationWillTerminate` `gpui_macos/src/platform.rs:98`) | gleich | `gpui-pre-0.3.8/src/app.rs:2594-2612`, `SHUTDOWN_TIMEOUT` `:78`; `gpui-pre-0.3.8/src/app/context.rs:200`; `gpui-pre-macos-0.3.8/src/platform.rs:107`, `:1472-1479` | ausstehend, Spike-Lauf | SP14 |
| `observe_window_activation` | vorhanden (`gpui/src/app/context.rs:444`) | gleich | `gpui-pre-0.3.8/src/app/context.rs:420` | ausstehend, Spike-Lauf | SP02, SP03 |
| `remove_window` | vorhanden (`gpui/src/window.rs:2187`) | gleich. Setzt nur `removed`, das Fenster schließt am Ende des Updates | `gpui-pre-0.3.8/src/window.rs:2324-2326` | ausstehend, Spike-Lauf | SP02 |
| `KeyBinding::load` mit Mapper | `use_key_equivalents` und `PlatformKeyboardMapper` (`gpui/src/keymap/binding.rs:48`) | gleich | `gpui-pre-0.3.8/src/keymap/binding.rs:48-66`; `App::keyboard_mapper` `gpui-pre-0.3.8/src/app.rs:1139`; `gpui-pre-macos-0.3.8/src/platform.rs:1144` | ausstehend, Spike-Lauf (der Test-Mapper der Headless-Tests sagt nichts über QWERTZ) | SP10 |
| Layoutwechsel zur Laufzeit | `on_keyboard_layout_change` (`gpui/src/app.rs:1016`; `gpui_macos/src/platform.rs:1359`) | gleich | `gpui-pre-0.3.8/src/app.rs:1144`; `gpui-pre-macos-0.3.8/src/platform.rs:156-159`, `:1482` | ausstehend, Spike-Lauf | SP10 |
| `HasWindowHandle` | `Window` und `MacWindow` (`gpui/src/window.rs:7098`; `gpui_macos/src/window.rs:2205`) | gleich. Der Handle trägt die `NSView`, das `NSWindow` (bzw. `NSPanel`) kommt über `[view window]` | `gpui-pre-0.3.8/src/window.rs:7434`; `gpui-pre-macos-0.3.8/src/window.rs:2410-2417` | ausstehend, Spike-Lauf | SP03, SP12 |
| `sharingType` | nicht vorhanden | nicht vorhanden, auch nicht in GPUI Kit. Nur über `HasWindowHandle` und objc2 in `platform/panel.rs` erreichbar | kein Treffer für `sharingType` oder `NSWindowSharing` in `gpui-pre-0.3.8` und `gpui-pre-macos-0.3.8` | ausstehend, Spike-Lauf | SP12 |
| `cx.hide` | `NSApp hide:` (`gpui/src/app.rs:1307`; `gpui_macos/src/platform.rs:598`) | gleich | `gpui-pre-0.3.8/src/app.rs:1472-1474`; `gpui-pre-macos-0.3.8/src/platform.rs:711-716` | ausstehend, Spike-Lauf | SP05 |
| `reduce_motion` | nur das Flag am `App`, die Plattform liest die Einstellung nicht (`gpui/src/app.rs:1059`) | Flag gleich, die macOS-Plattform liest weiter nichts. **Neu über GPUI Kit:** `gpui_base::init` liest `accessibilityDisplayShouldReduceMotion` einmal beim Start, `gpui_component::init` ruft es auf. Ein Wechsel bei laufender App kommt auf macOS nicht an, dafür gibt es `apply_system_reduce_motion` | `gpui-pre-0.3.8/src/app.rs:1222-1233`; kein Treffer für `reduce_motion` in `gpui-pre-macos-0.3.8`; `gpui-base-0.7.1/src/reduce_motion.rs:108-112`, `:65`, `gpui-base-0.7.1/src/lib.rs:231`, `gpui-component-0.7.1/src/lib.rs:134` | ausstehend, Spike-Lauf | P0.07 (DS-0) |
| AccessKit-Rollen | accesskit 0.24 und accesskit_macos 0.26 (Workspace-`Cargo.toml:515-516`), `SubclassingAdapter` (`gpui_macos/src/window.rs:2160`), `role`, `aria_label`, `aria_value` (`gpui/src/elements/div.rs:1268`, `:1290`, `:1384`) | gleich, dieselben Versionen | `gpui-pre-0.3.8/Cargo.toml:265-267`; `gpui-pre-macos-0.3.8/Cargo.toml:58-63`; `gpui-pre-macos-0.3.8/src/window.rs:2365`; `gpui-pre-0.3.8/src/gpui.rs:97-99`; `gpui-pre-0.3.8/src/elements/div.rs:1310`, `:1332`, `:1341`, `:1426`; `gpui-pre-0.3.8/src/element.rs:114` | ausstehend, Spike-Lauf (VoiceOver, Accessibility Inspector) | SP03 (VoiceOver), T3 |
| Mindest-macOS von gpui-pre | Shader für 10.15.7 übersetzt (`gpui_apple/build.rs:147`), Laufzeitweichen für 12 und 15 | gleich. Liegt weit unter der FerrisPass-Mindestversion 26 (K12), es braucht keine neue Entscheidung (Plan 20, 4.5) | `gpui-pre-apple-0.3.8/build.rs:138`; `gpui-pre-macos-0.3.8/src/platform.rs:847`; `gpui-pre-macos-0.3.8/src/window.rs:365` | nicht nötig | keiner |

### Was die Matrix für die Spikes heißt

- **SP03 bleibt nötig.** `canBecomeKeyWindow` ist in gpui-pre 0.3.8 weiter fest `YES`, und `activate` macht jedes Fenster Key. Ein nicht key-fähiges Panel geht nur über eine eigene Unterklasse oder `setBecomesKeyOnlyIfNeeded` per `HasWindowHandle` (Plan 20, SP03 a und b).
- **SP06 muss den Kaltstart prüfen.** URLs, die vor dem Setzen des Callbacks ankommen, gehen verloren. `on_open_urls` gehört deshalb an den `Application`-Builder vor `run`, nicht in den ersten Frame.
- **SP12 baut auf `HasWindowHandle`.** GPUI kennt `sharingType` nicht.
- **P0.07 (DS-0) hat eine Antwort:** GPUI Kit liest "Bewegung reduzieren" beim Start selbst, folgt einer Änderung aber nicht. Ob FerrisPass `apply_system_reduce_motion` beim Aktivieren erneut aufruft, entscheidet DS-0.

## Headless-UI-Tests auf dem CI-Runner

P0.02 soll klären, ob Headless-UI-Tests auf dem `macos-latest`-Runner laufen. Antwort aus dem Quelltext: Die Tests von FerrisPass brauchen kein Metal und keinen Fensterserver.

- `#[gpui::test]` baut einen `TestAppContext` auf `TestPlatform::new`, mit `NoopTextSystem` und ohne Renderer (`gpui-pre-0.3.8/src/app/test_context.rs:131`, `gpui-pre-0.3.8/src/platform/test/platform.rs:124-131`).
- Fenster bekommen dann einen `HeadlessAtlas` statt eines Metal-Atlas (`gpui-pre-0.3.8/src/platform/test/window.rs:90-93`).
- Metal braucht nur `HeadlessAppContext` mit `gpui_platform::current_headless_renderer` (`gpui-pre-0.3.8/src/app/headless_app_context.rs:82`, `gpui-pre-platform-0.3.8/src/gpui_platform.rs:111`). Das nutzt FerrisPass nicht. Für Pixel-Vergleiche in T3 wäre es der Weg, und erst dann stellt sich die Metal-Frage auf dem Runner.

Die Tests aus P0.01 (`tests/keymap.rs`, die Tests in `src/ui/widgets/secret_input.rs`) laufen lokal so grün. Den Beleg auf dem Runner liefert erst der erste CI-Lauf des Zweigs, der noch nicht gepusht ist.

## Spike-Ergebnisse

Jeder Spike trägt hier eine Zeile und einen Kurzbericht mit Messwerten ein (Plan 20, Kapitel 5, "Vorgehen für alle Spikes"). Ergebnis ist "bestanden", "gescheitert" oder "Ausweg gewählt". Die P-Pakete stehen in Plan 30, F4.

| Nr | Spike | P-Paket | Ergebnis | Messwert | Ausweg | Datum, Zweig |
|---|---|---|---|---|---|---|
| SP01 | Touch ID aus dem Panel, Fokus-Rückgabe, D2 | P0.04 (ohne D2), P0.09 (D2-Teil) | | | | |
| SP02 | Aktivierung nach dem Schließen in 1 s | P0.03 | | | | |
| SP03 | Nicht key-fähiges Panel | P0.08 | | | | |
| SP04 | Bedienungshilfen-Lesen in Browsern | P0.10 | | | | |
| SP05 | Panel bei ⌘H und geschlossenem Fenster | P0.03 | | | | |
| SP06 | URL-Schema und Dokumenttypen | P0.11 | | | | |
| SP07 | Status-Item mit Notch (a), Login-Item mit Starterkennung (b) | P0.05 (a), P0.11 (b) | | | | |
| SP08 | KeePassXC-Rundlauf mit CustomData | P0.12 | | | | |
| SP09 | Schrift | P0.07 (mit DS-0) | | | | |
| SP10 | Deutsches Tastaturlayout | P0.14 | | | | |
| SP11 | Ein LAContext für mehrere Tresore | P0.09 | | | | |
| SP12 | `sharingType` gegen Teams, Zoom, Webex | P0.13 | | | | |
| SP13 | Zwischenablage-Verläufe (a), Datenschutzabfrage beim Start (b) | P0.13 (a), P0.06b (b) | | | | |
| SP14 | Secure Event Input im GPUI-Passwortfeld | P0.06 | | | | |

### Kurzberichte

Noch keine. Je Spike ein Unterabschnitt `### SPnn Titel` mit Frage, Aufbau, Messwerten, Entscheidung und dem Verweis auf den Hindsight-Eintrag (Bank `sidehustle`).
