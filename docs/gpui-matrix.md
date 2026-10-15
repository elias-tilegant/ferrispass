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
| `KeyBinding::load` mit Mapper | `use_key_equivalents` und `PlatformKeyboardMapper` (`gpui/src/keymap/binding.rs:48`) | gleich. Der Mapper ersetzt nur Satzzeichen nach Apples lokalisierter Tabelle (Deutsch: `/` wird `ß`, `[` wird `ö`), Buchstaben bleiben Zeichen | `gpui-pre-0.3.8/src/keymap/binding.rs:48-66`; `App::keyboard_mapper` `gpui-pre-0.3.8/src/app.rs:1139`; `gpui-pre-macos-0.3.8/src/platform.rs:1144`; Tabellen `gpui-pre-macos-0.3.8/src/keyboard.rs:111` | headless belegt (SP10): Deutsch, Schweiz und US per `UCKeyTranslate` gelesen, ohne die Eingabequelle zu ändern, Dispatch im Headless-Fenster; die deutsche Tabelle stimmt mit dem Live-Mapper dieses Macs überein | SP10 |
| Layoutwechsel zur Laufzeit | `on_keyboard_layout_change` (`gpui/src/app.rs:1016`; `gpui_macos/src/platform.rs:1359`) | gleich. **Ersetzt nur den Mapper, schon geladene Bindungen behalten die alte Tabelle**; weder GPUI Kit noch gpui-base binden neu. Die Registry muss im Callback neu laden | `gpui-pre-0.3.8/src/app.rs:1144`, `:966-977`; `gpui-pre-macos-0.3.8/src/platform.rs:156-159`, `:1482-1486` | nicht nötig, Quelltext eindeutig (SP10) | SP10 |
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

Die Tests aus P0.01 (`tests/keymap.rs`, die Tests in `src/ui/widgets/secret_input.rs`) laufen lokal so grün. Der erste CI-Lauf des Zweigs (PR #2, 15.10.2026, Image `macos-26-arm64`) bestätigt das: Alle Tests laufen dort grün, die Headless-UI-Tests eingeschlossen.

SP09 hat den Metal-Weg lokal genutzt: `HeadlessAppContext::with_platform` mit dem Textsystem der headless erzeugten macOS-Plattform und `gpui_platform::current_headless_renderer()` (Feature `test-support` von gpui-pre-platform) rendert eine Ansicht ohne Fenster in ein Bild. Für Pixel-Vergleiche in T3 geht das auf dem Mac; ob der Runner Metal hat, bleibt offen.

## Spike-Ergebnisse

Jeder Spike trägt hier eine Zeile und einen Kurzbericht mit Messwerten ein (Plan 20, Kapitel 5, "Vorgehen für alle Spikes"). Ergebnis ist "bestanden", "gescheitert" oder "Ausweg gewählt". Die P-Pakete stehen in Plan 30, F4.

| Nr | Spike | P-Paket | Ergebnis | Messwert | Ausweg | Datum, Zweig |
|---|---|---|---|---|---|---|
| SP01 | Touch ID aus dem Panel, Fokus-Rückgabe, D2 | P0.04 (ohne D2), P0.09 (D2-Teil) | | | | |
| SP02 | Aktivierung nach dem Schließen in 1 s | P0.03 | | | | |
| SP03 | Nicht key-fähiges Panel | P0.08 | | | | |
| SP04 | Bedienungshilfen-Lesen in Browsern | P0.10 | | | | |
| SP05 | Panel bei ⌘H und geschlossenem Fenster | P0.03 | vorbereitet | - | - | 15.10.2026, `spike/p0-03-quick-panel` (d96e026), Lauf mit Elias ausstehend |
| SP06 | URL-Schema und Dokumenttypen | P0.11 | vorbereitet | Parser-Tests 3 von 3 | - | 15.10.2026, `spike/p0-11-url-scheme` (9f08037), Lauf mit Elias ausstehend |
| SP07 | Status-Item mit Notch (a), Login-Item mit Starterkennung (b) | P0.05 (a), P0.11 (b) | | | | |
| SP08 | KeePassXC-Rundlauf mit CustomData | P0.12 | Ausweg gewählt: Merge-Filter B35 (CLI-Durchgang; GUI-Durchgang offen) | Markenwerte bytegleich mit keepassxc-cli 2.6.6, 2.7.12, 2.8.0-beta1; Item-Zeiten gehen verloren; ohne Filter 3 Eintrags- und 1 Gruppenkonflikt je KeePassXC-Speichern, mit Filter 0 (6 von 6 Paaren, 10 von 10 Runden) | B35 wie im Kurzbericht, Uhr der Marke im Wert | 15.10.2026, `spike/p0-12-keepassxc-roundtrip` (ad3209a, d93dfff) |
| SP09 | Schrift | P0.07 (mit DS-0) | bestanden, Medium nur mit Gewicht 530 | `.SystemUIFont` weicht 0,00 bis 0,11 % von der CoreText-Systemschrift ab (11 bis 28 pt); der heutige Stack ist Helvetica (-4,63 % bei 13 pt); `tnum` macht alle Ziffern 8,112 breit | keiner nötig; "SF Pro direkt setzen" trägt nicht | 15.10.2026, `spike/p0-07-font` (bf56eea) |
| SP10 | Deutsches Tastaturlayout | P0.14 | bestanden | 35 von 35 App-Kürzeln je Layout (Deutsch, Schweiz, US) lösen im Headless-Fenster aus; 0 von 19 Option-Zeichen werden von einem Kürzel genommen | ⌘↑ statt ⌘[ bleibt | 15.10.2026, `spike/p0-14-keyboard-layout` (905bf11) |
| SP11 | Ein LAContext für mehrere Tresore | P0.09 | | | | |
| SP12 | `sharingType` gegen Teams, Zoom, Webex | P0.13 | | | | |
| SP13 | Zwischenablage-Verläufe (a), Datenschutzabfrage beim Start (b) | P0.13 (a), P0.06b (b) | b vorbereitet; a offen (Gerätesitzung "Fremde Apps") | - | - | 15.10.2026, b: `spike/p0-06b-pasteboard-privacy` (747a94d), Lauf mit Elias ausstehend |
| SP14 | Secure Event Input im GPUI-Passwortfeld | P0.06 | vorbereitet | - | - | 15.10.2026, `spike/p0-06-secure-input` (b06bb8a), Lauf mit Elias ausstehend |

### Kurzberichte

Je Spike ein Unterabschnitt `#### SPnn Titel` mit Frage, Aufbau, Messwerten, Entscheidung und dem Verweis auf den Hindsight-Eintrag (Bank `sidehustle`). Laufversuche, die ein Fenster, ein globales Kürzel oder einen Systemdialog brauchen, sind vorbereitet und laufen gebündelt mit Elias (Anleitung `~/Documents/elias/hobby/ferrispass-spikes/SPIKE-LAUF.md`).

#### SP08 KeePassXC-Rundlauf mit CustomData (P0.12)

- **Aufbau:** `examples/spike_12.rs` schreibt mit keepass-rs eine KDBX-4.1-Datei mit Marken an der Gruppe `Kunden` (`FerrisPass.Role`), an zwei Einträgen (`FerrisPass.Type`, `FerrisPass.RotationSince`, `FerrisPass.Via`, eine Marke ohne Item-Zeit) und im Meta-Block (`FerrisPass.Fav.<Tresor>.<UUID>`). `keepassxc-cli edit` ändert einmal einen anderen und einmal den markierten Eintrag, je mit 2.7.12 (installiert), 2.6.6 (vorige Minor) und 2.8.0-beta1. Verglichen wird das entschlüsselte XML Byte für Byte und mit `merge::diff` und `apply_picks` der App. Ohne Fenster; der GUI-Durchgang steht aus.
- **Marken:** Die Werte sind in allen drei Versionen bytegleich, auch Werte, die keepass-rs als Binärwert liest. Die Item-`LastModificationTime` geht verloren: KeePassXC schreibt sie für Eintrag und Gruppe nie, für Meta nur in KDBX 4.1, und `keepassxc-cli` hat die 4.1-Datei in allen drei Versionen als 4.0 gespeichert. KeePassXC 2.6.6 liest und schreibt die 4.1-Datei ohne Fehler.
- **`_LAST_MODIFIED`:** KeePassXC hängt an jeden nicht leeren CustomData-Block diesen Schlüssel mit lokalisiertem Datumstext und stempelt ihn bei jedem Laden neu, ohne die Bearbeitungszeit des Objekts zu bewegen (KeePassXC 2.7.12 `CustomData::set` ruft `updateLastModified` auch für das gelesene Item). Gemessen: 30 von 30 Neustempelungen in 10 Runden.
- **Merge heute (0.9.x):** Nach jedem KeePassXC-Speichern 3 Eintragskonflikte und 1 Gruppenkonflikt bei unveränderten Einträgen, in beide Richtungen. Ursachen: `_LAST_MODIFIED` und die verlorenen Item-Zeiten an markierten Objekten, außerdem **ohne Marken**: keepass-rs legt neue Einträge ohne Auto-Type-Block an (`create_entry` ebenso), KeePassXC schreibt beim nächsten Speichern den Standardblock, ohne die Zeit zu bewegen, und `metadata_differences` vergleicht `autotype` als Ganzes.
- **Reine Markenänderung:** ohne bewegte Eintrags- oder Gruppenzeit eine Entscheidung (M1, M3), mit bewegter Zeit automatisch (M2, M4). Wird eine Marke mit bewegter Zeit gesetzt und dort später der Eintrag bearbeitet, gewinnt die neuere Seite den ganzen Eintrag und die Markenänderung ist still weg (M6).
- **B35 als Prototyp (`fdiff`):** An Eintrag und Gruppe `_LAST_MODIFIED` und `KPXC_RANDOM_SLUG` sowie alle `FerrisPass.*` aus dem Vergleich nehmen und Item-Zeiten ignorieren; dazu einen fehlenden Auto-Type-Block wie KeePassXCs Standard lesen. Ergebnis: 0 Konflikte in 6 von 6 Rundlauf-Paaren, 0 Entscheidungen in 10 von 10 Churn-Runden, M1 und M3 ohne Entscheidung.
- **Entscheidung:** Ausweg Merge-Filter B35, eindeutig definiert. Vorschlag für P2.01: Marken pro Schlüssel getrennt mergen, mit der Uhr im Wert (etwa `v1|<Unix-Sekunden>|<Inhalt>`, Löschen als Grabstein mit leerem Inhalt), weil keine Item-Zeit KeePassXC übersteht; das Format ist zugleich kein gültiges Base64. Marken setzen bewegt die Eintragszeit nicht. Markenwerte als XML-Text lesen (String oder Base64 des Binärwerts).
- **Für Elias:** `_LAST_MODIFIED` an Eintrag und Gruppe und der Auto-Type-Standard erzeugen schon in 0.9.x Scheinkonflikte, sobald ein Eintrag CustomData trägt (etwa KeePassXC-Browser-Schlüssel) oder in FerrisPass angelegt wurde. Ob das vor 0.10.0 behoben wird, ist offen.
- Korpus: `ferrispass-spikes/sp08/run-20261015-103307`. Hindsight: `ferrispass-spike-sp08-2026-10`.

#### SP09 Schrift (P0.07)

- **Aufbau:** `examples/spike_07.rs` erzeugt die macOS-Plattform headless und startet sie nie (kein `NSApplication`, kein Fenster) und misst mit ihrem CoreText-Textsystem. Referenz statt TextEdit: eine CoreText-Zeile mit `kCTFontUIFontSystem`, also `.SFNS-Regular`, die Systemschrift, die TextEdit setzt. Das Bild `sp09/out/spike_07.png` kommt aus GPUIs Headless-Renderer.
- **Auflösung:** Der Stack in `src/ui/theme.rs:23-25` ist für GPUI ein Familienname, wird nicht gefunden und fällt auf Helvetica (gleiche FontId). `"SF Pro"` und `"SF Pro Text"` gibt es als Familie nicht und fallen ebenfalls auf Helvetica. `.SystemUIFont` löst auf `.SFNS` auf.
- **Breite des Testsatzes** gegen die CoreText-Systemschrift: `.SystemUIFont` -0,10 % (11 pt), 0,00 % (13 pt), +0,11 % (17 pt), +0,11 % (20 pt), -0,11 % (28 pt). Der heutige Stack: -6,89 %, -4,63 %, -0,61 %, +1,95 %, +4,49 %.
- **Gewichte:** 400, 600 und 700 sind eigene Schnitte. 500 landet auf Regular, 800 und 900 beide auf Black, obwohl CoreText alle neun Schnitte führt. Ursache: font-kit bildet den Gewichts-Trait stückweise auf CSS ab (`FONT_WEIGHT_MAPPING`), SF Medium liegt dort bei etwa 530. `FontWeight(510)` bis `FontWeight(530)` trifft Medium, 750 bis 780 Heavy. Bei den eingebetteten Atkinson-Dateien trifft 500 richtig Medium.
- **`tnum`:** Ziffern 0 bis 9 ohne `tnum` 5,954 bis 8,290 pt breit, mit `tnum` alle 8,112 (13 pt).
- **Atkinson Hyperlegible Mono** (OFL 1.1 ohne Reserved Font Name, upstream `googlefonts/atkinson-hyperlegible-next-mono` 154d503): die statischen TTF laden per `add_fonts`, alle Zeichen 8,216 breit. Die variable `[wght]`-Datei allein liefert nur einen Schnitt, also die statischen Dateien einbetten.
- **Umlaute:** keine fehlende Glyphe für ÄÖÜäöüß€„“, zerlegte und zusammengesetzte Schreibweise gleich breit und gleich viele Glyphen.
- **Entscheidung:** bestanden. P1a.09b setzt `font_family` auf `.SystemUIFont`, Medium der Systemschrift als `FontWeight(530.0)` mit einem Headless-Test, der Medium von Regular und Semibold unterscheidet. Hindsight: `ferrispass-spike-sp09-2026-10`.

#### SP10 Deutsches Tastaturlayout (P0.14)

- **Aufbau:** `examples/spike_14.rs` liest die Layouts Deutsch, Schweiz und US per `TISCreateInputSourceList` und `UCKeyTranslate`, ohne eine Eingabequelle auszuwählen, und rechnet mit GPUIs eigener Übersetzung (`parse_keystroke`, `gpui-pre-macos-0.3.8/src/events.rs:421-482`), welche Taste welchen Keystroke ergibt. Die Kürzel aus Zielbild 3.4 lädt `KeyBinding::load` mit GPUIs Tabellen. Die Tests drücken jedes Kürzel so, wie es eine Person mit dem Layout drückt, durch ein Headless-Fenster mit FerrisPass- und GPUI-Kit-Keymap (5 von 5 grün; die Mutationsprobe mit einer zusätzlichen Bindung `alt-l` und dem Laden ohne `use_key_equivalents` machte drei Tests rot). HIToolbox bricht ab, wenn TIS nicht auf dem Hauptthread läuft, daher `--test-threads=1`.
- **Ergebnis:** 35 von 35 App-Kürzeln lösen auf allen drei Layouts aus. Buchstaben binden nach Zeichen, nicht nach Position: ⌘Y und ⌘Z liegen auf QWERTZ vertauscht, wie in jeder Mac-App. ⌃⌥⌘P und ⌃⌥⌘V laufen über Carbon nach Keycode, also nach Position; P und V liegen auf allen drei gleich.
- **⌘/:** mit `use_key_equivalents` Keycap und Taste ⌘ß (Deutsch), ⌘' (Schweiz), ⌘/ (US). Ohne: ⇧⌘7 auf Deutsch und Schweiz. **⌘[** ist ohne Key-Equivalents auf Deutsch und Schweiz nicht erreichbar, mit ihnen ⌘ö; ⌘↑ aus dem Zielbild gilt überall.
- **Texteingabe:** 0 von 19 Option-Zeichen (Deutsch ⌥L = @, ⌥E = €, ⌥5 bis ⌥9 Klammern und |, ⌥⇧7 = \; Schweiz ⌥G = @) werden von einem Kürzel genommen, alle kommen im Feld an. Weil `Keystroke::should_match` einen Keystroke mit `key_char` auch gegen eine Bindung ohne ⌥ prüft, darf die Registry kein `alt-<Buchstabe>` oder `alt-<Ziffer>` ohne ⌘ oder ⌃ binden.
- **Keycaps:** GPUIs `Display` zeigt ⌃ als `^`, ordnet ⌘ vor ⇧ und schreibt `enter` aus. Der Keycap-Baustein (P1a.09b) setzt die Glyphen selbst in Apples Reihenfolge ⌃⌥⇧⌘.
- **Layoutwechsel:** siehe Matrix; die Registry (P1a.16) bindet in `on_keyboard_layout_change` neu.
- **Entscheidung:** bestanden. Hindsight: `ferrispass-spike-sp10-2026-10`.

#### SP05, SP06, SP13 b, SP14 (vorbereitet)

Beispiel, Ablaufskript und Protokollvorlage liegen auf den Zweigen `spike/p0-03-quick-panel` (`examples/spike_03.rs`, `spike/run_sp05.sh`), `spike/p0-11-url-scheme` (`examples/spike_11.rs`, `spike/run_sp06.sh`, Parser mit Unit-Tests nach V15), `spike/p0-06b-pasteboard-privacy` (`examples/spike_06b.rs`, `spike/run_sp13b.sh`) und `spike/p0-06-secure-input` (`examples/spike_06.rs`, `examples/spike_06_tap.rs`, `spike/run_sp14.sh`). Alle bauen, keiner ist gestartet. Die Läufe brauchen ein Fenster, das globale Kürzel ⌃⌥⌘P, LaunchServices oder einen möglichen Systemdialog; SP14 braucht Eingabeüberwachung für das Terminal. SP13 a läuft in der Gerätesitzung "Fremde Apps lesen und aufnehmen".
