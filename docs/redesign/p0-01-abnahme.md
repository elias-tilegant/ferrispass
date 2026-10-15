# Abnahme P0.01: Migration auf GPUI Kit (Stufe 0)

Stand 14.10.2026, Zweig `p0-01-gpui-kit`, Ausgang `master` `d3a7e6d` (v0.9.2). Gliederung nach Plan 20, 4.9. Was nur von Hand geht, steht als Vorlage in Abschnitt 7.

## 1 Gates (4.9 Punkt 1)

| Gate | Ergebnis |
|---|---|
| `cargo fmt --all --check` | grün |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | grün (einzige Meldung: die Future-Incompat-Notiz zu `block` 0.1.6, wie in v0.9.2) |
| `cargo test --locked` | grün, kein neuer `#[ignore]` |
| `cargo build --locked --bin ferrispass-cli` | grün |
| Gedankenstrich-Scan über `git ls-files` | leer |
| Gedankenstrich-Scan über die Commit-Messages | beim Commit |
| `cargo audit` | **rot, aber vorbestehend:** rustls 0.23.45 (RUSTSEC-2026-0346 bis -0349, Fix 0.23.46, im Release-Binary) und quinn-proto 0.11.17 (RUSTSEC-2026-0338, -0340, -0343, Fix 0.11.18, nicht im macOS-Release-Graph). Der Lock von `master` hat genau dieselben Meldungen. Die Anhebung gehört in einen eigenen Commit wie `f441593` |

**Die Gedankenstrich-Gates in der CI liefen bis hierher ins Leere.** Die Runner haben BSD-grep, das `-P` nicht kennt. Beide Scans endeten mit Exit 2, und beide Schritte werteten das als sauber. `ci.yml` baut das Muster jetzt aus den UTF-8-Bytes, sucht mit `-E` und prüft sich vorher selbst: Findet der Selbsttest den eingebauten Gedankenstrich nicht, schlägt der Job fehl.

## 2 Release-Build (4.9 Punkt 2)

Gemessen mit `/usr/bin/time -l cargo build --release --locked` (LTO fat, `codegen-units = 1`), 12 Kerne, 36 GB, macOS 27.0.1, SDK 27.0. Der Vorher-Wert ist auf `d3a7e6d` neu gebaut, weil das vorhandene `target/release` älter als v0.9.2 war.

| | vorher (v0.9.2, warm) | nachher kalt (neue Kit-Crates) | nachher warm |
|---|---|---|---|
| Dauer (real) | 123,7 s | 147,4 s | 122,9 s |
| Spitzenspeicher (max RSS) | 3,15 GiB | 3,52 GiB | 3,33 GiB (+5,5 %) |
| `ferrispass` | 16 482 752 B | 17 960 112 B (+9,0 %) | 17 960 112 B |
| `ferrispass-cli` | 4 202 416 B | 4 218 848 B (+0,4 %) | 4 218 848 B |

**Inspector:** nicht im Release-Build. Das Feature `inspector` steht nirgends im Feature-Graph des Release-Builds (`cargo tree -e features,normal,build`). `strings` findet weder `ToggleInspector` noch `inspector::` noch `DivInspector`, nur Wörter aus der zxcvbn-Liste und den Pfad `icons/inspector.svg`. Gegenprobe: Das Debug-Binary enthält `inspector::ToggleInspector`.

**Mindestversion im Binary:** Ohne `MACOSX_DEPLOYMENT_TARGET` schreibt rustc `minos 11.0`. `scripts/build-mac.sh` setzt das Ziel auf `MIN_MACOS` (jetzt 26.0) und bricht ab, wenn das ausgewählte SDK älter als die Mindestversion ist. Ob der Release-Runner (`macos-latest`) ein SDK ab 26 hat, zeigt erst der erste Release-Lauf.

## 3 Lieferkette (4.9 Punkt 3)

Vorgehen und Ergebnis stehen in [release-process.md, Abschnitt "GPUI supply chain"](../release-process.md#gpui-supply-chain-on-every-gpui-bump), das Skript ist `scripts/verify-gpui-pre.sh` (Endlauf Exit 0). Kurz:

- Alle Prüfsummen passen zum Lock. Alle 22 Zed-Crates nennen zed@279fe07, und der Commit liegt auf Zeds `main`.
- 19 Crates sind quellgleich mit Zed. Drei tragen Patches aus Longbridges öffentlichem Republish-Skript `script/bump-gpui.ts`: `actions!` leitet `$crate::Action` ab, `gpui-pre-apple` liest die Shader-Quellen aus `vendor/` (byte-gleich mit Zed), und `gpui-pre-macros` schreibt erzeugte Pfade auf die Fassade `gpui_kit::` um, aber nur, wenn das Manifest der kompilierten Crate `gpui-kit` nennt. Die GK-Recherche 2.2 ("no behavioral patches") ist damit überholt.
- Die gpui-pre-Crates haben kein `.cargo_vcs_info.json`. Den Zed-Stand liest das Skript aus `[package.metadata.gpui-pre]`, nicht wie in 4.6 angenommen aus `git.sha1`.
- Die GPUI-Kit-Crates sind byte-gleich mit dem Tag v0.7.1. Netz kommt nur auf wasm vor. Auf macOS gibt es kein neues `unsafe` gegenüber dem alten Git-Stand, das Speech-Modul liegt hinter einem abgeschalteten Feature.
- gpui-pre und gpui-component veröffentlicht dasselbe crates.io-Konto.

## 4 Keine Verhaltensänderung (4.9 Punkt 4)

### Tastenbelegung (T0.2)

`tests/snapshots/keymap.txt` hält alle Bindungen in Registrierungsreihenfolge fest (`tests/keymap.rs`). Der alte Stand wurde zweimal erhoben, statisch aus `b004e59` und `actions.rs` und zur Laufzeit über ein temporäres Example. Beide Wege ergaben dasselbe.

| | alt | neu |
|---|---|---|
| Bindungen ohne Inspector | 130 | 154 |
| davon FerrisPass (`actions::init`) | 19 | 19, unverändert in Tasten, Kontext und Reihenfolge |

- **Entfallen (2), nur umbenannt:** Kontext `Dialog`, `escape` von `dialog::CancelDialog` auf `ui::Cancel` und `enter` von `dialog::ConfirmDialog` auf `ui::Confirm`. FerrisPass nutzt den Kit-Dialog nicht.
- **Neu (24):**
  - Kontext `Input`: `alt-cmd-up` und `alt-cmd-down` (Mehrfach-Cursor, nur bei mehrzeiligen Feldern wirksam, FerrisPass hat keine) und `cmd-shift-f` Replace (nicht durchsuchbare Felder reichen es weiter, FerrisPass belegt die Taste nicht).
  - Kontext `Popover`: `enter` und `space` auf `ui::Confirm`. FerrisPass rendert keinen Popover, das Gruppen-Kontextmenü läuft über `PopupMenu`.
  - Außerdem `escape` im ColorPicker sowie TimeField (8), Command (4) und Carousel (6). FerrisPass nutzt diese Komponenten nicht.
- **Reihenfolge:** Die Kit-Module registrieren in anderer Reihenfolge. Zwischen Kontexten entscheidet die Tiefe, und innerhalb gemeinsamer Kontexte ist die relative Reihenfolge gleich geblieben.
- **Inspector:** Debug-Builds registrieren `alt-cmd-i` für `inspector::ToggleInspector`, alt wie neu. Der Test nimmt genau diese eine Zeile heraus, und jede andere Inspector-Bindung lässt ihn scheitern.

**Die Verhaltensänderung, die der Snapshot nicht zeigt: ⌘F in einem Feld.** Die Handler für Enter, Escape, ⌘C, ⌘F und ⌘Backspace wurden alt gegen neu gelesen. Bei vier ist das Verhalten gleich geblieben. Bei ⌘F hat es sich geändert: Bis 0.5 hat ein nicht durchsuchbares Feld `input::Search` verschluckt, seit 0.7 reicht es die Aktion per `cx.propagate()` weiter, und die Bindung `cmd-f` auf `FocusSearch` greift. **Entscheidung Elias (15.10.2026): das neue Verhalten bleibt**, ⌘F sucht immer, wie es auf dem Mac üblich ist. Eine Lücke wurde dabei geschlossen: Bei offenem Overlay (Eintragseditor, Gruppe, Konflikt, Einstellungen, Switcher und andere) ist das Suchfeld nicht gezeichnet oder liegt hinter einem Modal. Ohne Schutz wäre der Fokus aus dem Editor in ein unsichtbares Feld gesprungen. `AppShell::focus_search` tut deshalb nichts, solange ein Overlay offen ist (`Overlay::is_active`). Der Test `cmd_f_focuses_the_search_from_the_shell_and_from_a_text_field` belegt das neue Verhalten. Der Overlay-Schutz steht im manuellen Protokoll, weil der Test-Stand-in kein Overlay kennt. Die Release-Notiz von 0.9.3 nennt die Änderung in einem Satz.

Dispatch-Tests, headless und ohne Metal:
- `cmd_c_copies_the_password_from_the_list_but_not_from_a_text_field`: Mit dem Fokus auf der Shell löst ⌘C `CopyPassword` aus. Im fokussierten Feld kopiert ⌘C nur die Feldauswahl, mit und ohne Auswahl kommt nie `CopyPassword` an.
- `cmd_f_focuses_the_search_from_the_shell_and_from_a_text_field`: siehe oben.

### Geschützte Felder (T0.1)

In `src/ui/widgets/secret_input.rs`, headless mit TestPlatform:
- Die Kontrolle mit einem ungeschützten Feld zeigt für jeden Exportweg einzeln, dass der Aufbau das Feld erreicht. Die Wege sind ⌘A ⌘C, ⌘A ⌘X sowie Select All mit Copy und Select All mit Cut, wie sie das Bearbeiten-Menü auslöst. Nach jedem Weg muss das Feld noch fokussiert sein.
- (a) Geschützt und maskiert: Kein Weg schreibt etwas in die Zwischenablage, der Wert bleibt erhalten.
- (b) Geschützt und per Auge (`toggle_masked`, der Pfad von `mask_toggle`) aufgedeckt: ebenso.
- **Mutationsprobe:** Ohne den Copy/Cut-Abfang in `SecretInput::render` wird (b) rot (das Geheimnis landet in der Zwischenablage), (a) bleibt grün, weil die Bibliothek im maskierten Zustand selbst blockt.
- Das Kontextmenü des geschützten Felds kommt aus `protected_context_menu()` und enthält nur Paste und Select All. Ob `render` es tatsächlich an `Input::context_menu` übergibt, lässt sich headless nicht beobachten, weil `NativeMenu::show` direkt zu AppKit geht. Diese Hälfte bleibt im manuellen Protokoll.

### Weitere geprüfte Stellen

- **K3, Toasts:** Die Vorgaben sind gleich geblieben: TopRight, dieselben Ränder, Zustellung im Fenster. Die Ebene liegt weiter über dem Overlay. Die Position prüft Elias im manuellen Protokoll, sie ist die einzige erlaubte sichtbare Abweichung neben K12.
- **K6, Theme:** `apply_choice` ruft erst `Theme::change` bzw. `Theme::sync_system_appearance`, danach in einem eigenen Aufruf `Theme::update` mit der Palette. Explizit gesetzte Schriftfamilien bleiben stehen.
- **Kontextmenü:** Lief schon im alten Stand über `NativeMenu`. Ein eigener Builder ersetzt das Standardmenü vollständig.

## 5 Codex-Gate (4.9 Punkt 5)

Drei getrennte Läufe mit `gpt-5.6-sol`, `model_reasoning_effort=xhigh`, read-only, ohne cargo.

**a) Zwischenablage, geschützte Felder:** kein Leck, weder maskiert noch per Auge aufgedeckt. Maskiert scheitert `is_copyable()` vor jedem Schreiben in die Zwischenablage. Aufgedeckt halten die `capture_action`-Handler von `SecretInput` Copy und Cut an, bevor die `on_action`-Handler der Bibliothek laufen. Das Kontextmenü enthält nur Paste und Select All. Das Touch-Overlay bietet Copy und Cut maskiert nicht an, aufgedeckt laufen sie über dieselben Aktionen und damit in den Abfang. Ziehen erweitert nur die interne Auswahl, ein Ziehen nach außen gibt es nicht. In gpui-pre gibt es keinen Services-Pfad (`writeSelectionToPasteboard`) für das Eingabefeld. Keine Befunde.

**b) Lieferkette:** Die drei Quell-Patches ändern laut Codex nichts Sicherheitsrelevantes. Am Skript fand Codex Lücken, durch die Exit 0 nicht bewies, was er versprach. Alle sind behoben:
- Archive werden jetzt vor dem Auspacken Eintrag für Eintrag geprüft und vom Helfer selbst geschrieben. Erlaubt sind nur Dateien und Verzeichnisse unter `<crate>-<version>/`, ohne Links, Geräte, absolute Namen, `..` oder doppelte Namen. Jeder Schritt in `fetch_crate` prüft seinen Status selbst, weil der Aufruf in einem `||` steht und `errexit` dort nicht greift.
- Was upstream liegt und im Paket fehlt, ist jetzt ein Befund und kein Hinweis mehr.
- Befunde an Dateien tragen den vollen SHA-256 beider Seiten. Ein geprüfter Patch passt also nicht mehr, sobald sich upstream etwas ändert.
- Abhängigkeiten werden mit allen Feldern verglichen (`registry`, `artifact`, `lib` usw.). Eine veröffentlichte Abhängigkeit muss von crates.io kommen, und als gelockerter Pin zählt nur `X` oder `^X` statt `=X`.
- Die Manifeste der Ersatz-Crates (zed-font-kit, zed-scap, gpui-pre-reqwest) und der GPUI-Kit-Crates werden wie das veröffentlichte `Cargo.toml` verglichen. Bisher lagen sie nur zum Lesen bei.
- Veraltete Einträge in `REVIEWED_FINDINGS` lassen den Lauf scheitern.
- Neu: Der Feature-Graph aus `cargo metadata` darf weder Inspector, Speech noch GitHub-Download einschalten, und gpui-fast, gpui-shell und gpui-webview dürfen nicht im Graph stehen. GPUI Kit muss von `EXPECTED_KIT_REV` (87d10ae5) kommen, ein verschobener Tag fällt auf. `curl` läuft mit `-q`, also ohne `~/.curlrc`.

Negativproben: Ein bösartiges Archiv mit `..`, absolutem Namen, Symlink, Hardlink, FIFO, doppeltem Namen oder einem Namen außerhalb des Crate-Verzeichnisses wird abgelehnt, und es wird nichts geschrieben. Ebenso fallen auf: eine zusätzliche Abhängigkeit in einem Ersatz-Crate, ein `registry`-Feld, ein Pin mit `>=`, eine geänderte Upstream-Datei unter einem geprüften Patch und eine Datei, die nur upstream liegt. Der Endlauf endet mit Exit 0.

Zwei Fragen konnte Codex nicht abschließen:
- **zed-font-kit fordert `dirs` 5 statt 6:** geklärt. Unter macOS ruft font-kit nur `dirs::home_dir` auf (`src/sources/fs.rs`, `default_font_directories`). `mac.rs` in dirs 5.0.1 und 6.0.0 ist identisch, ebenso `home_dir` in dirs-sys 0.4.1 und 0.5.0 (`$HOME`, sonst `getpwuid_r`). dirs 5.0.1 stand schon im Lock von `master`. Der Befund steht mit dieser Begründung in `REVIEWED_FINDINGS`.
- **Verhält sich GPUI Kit 0.7.1 wie der alte Git-Stand b004e59?** Das kann ein Quellvergleich nicht beweisen. Belegt wird es durch T0.1, T0.2, die Prüfung von K1 bis K12 im Review und das manuelle Protokoll (Abschnitt 7). Einen vollständigen Diff von 0.5.2 nach 0.7.1 gibt es nicht, er wäre zu groß für ein sinnvolles Review. Die Features im macOS-Release-Build stammen aus `cargo tree -e features,normal,build`: `gpui-component` und `gpui-base` laufen ohne Features, `gpui-pre` mit `default`, `font-kit`, `wayland`, `windows-manifest` und `x11`. wayland und x11 sind Linux-Plattformcode und werden auf macOS nicht gebaut.

Ein Befund wurde abgelehnt: Der Makro-Patch schreibt auch Stringliterale um. Das gilt nur, wenn das Manifest `gpui-kit` nennt, bei FerrisPass also nicht. Ohne `gpui-kit` gibt der Patch jedes Token unverändert weiter, und ändern lässt sich der Code der fremden Crate ohnehin nicht. Die Doku beschreibt die Umschreibung jetzt genau.

**c) Theme (K6):** gleiche Farben, Radien und Schriftfamilien, beim Start ohne Fenster wie beim Live-Wechsel. Die Zuweisungen sind dieselben wie in `master`. `Theme::change` lädt das registrierte Theme in beiden Versionen neu, und erst danach setzt `apply` die Palette. Ein zweites `Theme::update` lädt keine Vorgaben nach. Es gleicht die Tokens ab, synchronisiert Base und frischt alle Fenster auf. Explizit gesetzte Schriftfamilien bleiben stehen. Offen blieb bei Codex nur, ob die eingebauten Themes Verläufe enthalten, die beim Abgleich erhalten blieben. `default-theme.json` und `default-colors.json` enthalten keinen Verlauf, und FerrisPass registriert keine eigenen Theme-Dateien. Keine Befunde.

## 6 Abweichungen vom Plan

- **Dev-Abhängigkeiten:** Freigegeben war `gpui-kit =0.7.1` mit `test-support`. Das geht nicht: `gpui-pre-macros` 0.3.8 schreibt jeden erzeugten `gpui::`-Pfad auf `gpui_kit::` um, sobald das Manifest `gpui-kit` nennt, Dev-Abhängigkeiten eingeschlossen, und der normale Bibliotheks-Build scheitert dann. Stattdessen stehen `gpui` (gpui-pre) und `gpui-component` jeweils mit `test-support` unter `[dev-dependencies]`, also die Features, die `gpui-kit/test-support` einschaltet. **Braucht Elias' OK**, weil die Tabelle in Plan 20, 4.5 zwei andere Zeilen nennt. Für Variante B heißt das: Sobald `gpui-kit` eine Abhängigkeit ist, laufen alle gpui-Makros über die Fassade.
- **proptest** steht schon jetzt im Lock, transitiv über `gpui-pre/test-support`, und kommt nur in Testbuilds vor. Die Entscheidung T0.4 bleibt davon unberührt.
- **gpui-base und gpui-component-macros** sind per `cargo update -p ... --precise 0.7.1` auf 0.7.1 festgehalten, nur durch den Lock (Kommentar in `Cargo.toml`). Ein schlichtes `cargo update` hebt sie an, und gpui-base 0.7.2 bringt gpui-fast in den Lock.
- **Kontextmenü des Passwortfelds:** entsteht aus `protected_context_menu() -> gpui::Menu` und wird mit `NativeMenu::from` umgewandelt, damit ein Test die Einträge lesen kann. Die Einträge sind dieselben.
- **README, Lizenzabschnitt:** Die Begründung "ztracing / zlog (both GPL-3.0-or-later)" stimmt nicht. Beide sind Apache-2.0, in Zed 1870e26 wie in gpui-pre 0.3.8. Den Satz formuliert Elias neu, die Lizenz bleibt GPL-3.0-or-later.

## 7 Manuelles Protokoll Teil 0 (Vorlage für Elias)

Alt ist die installierte v0.9.2, neu ist der Build dieses Zweigs (`scripts/build-mac.sh --skip-notarize`, Ergebnis `dist/FerrisPass.app`). Die alte App beenden, bevor die neue startet, denn beide nutzen dieselbe Bundle-ID und dieselben Einstellungen.

**Mindestversion (K12)**
- [ ] `plutil -extract LSMinimumSystemVersion raw dist/FerrisPass.app/Contents/Info.plist` zeigt `26.0`
- [ ] Start auf macOS 26 (VM oder Kollegen-Mac) und auf macOS 27
- [ ] `dist/FerrisPass.app/Contents/Resources/Licenses/` enthält `ISC-Lucide.txt` und `OFL-JetBrainsMono.txt`

**Entsperren und Sperren**
- [ ] mit Passwort; mit Touch ID inklusive Autoprompt nach Auto-Lock (`f956864`); Argon2-Kostenfrage bei einem teuren KeePassXC-Tresor
- [ ] Auto-Lock; Sperre durch Bildschirmsperre und Ruhezustand

**Toasts (K3)**
- [ ] Ein Fehler-Toast bleibt stehen, ein Erfolgs-Toast blendet aus
- [ ] Position alt gegen neu vergleichen und notieren

**Darstellung (K6)**
- [ ] Theme System, Hell und Dunkel, jeweils mit Live-Wechsel bei laufender App; ⌘⇧D wechselt wie vorher
- [ ] Schrift unverändert (der SF-Pro-Fix gehört zu SP09)
- [ ] Icons: Check, Plus, Folder, Chevron hoch, runter und rechts, Settings, Close, Star, StarFill, Eye, EyeOff
- [ ] Scrollbalken in Liste und Einstellungen wie vorher

**Passwortfeld**
- [ ] Rechtsklick ins geschützte Feld zeigt nur Paste und Select All, maskiert und aufgedeckt
- [ ] ⌘C und ⌘X kopieren nichts, maskiert und aufgedeckt (danach in TextEdit einfügen)
- [ ] Bearbeiten-Menü > Kopieren im aufgedeckten geschützten Feld kopiert nichts

**Bearbeiten-Menü und Kit-Komponenten**
- [ ] Undo, Redo, Cut, Copy und Paste in normalen Feldern
- [ ] Generator: Slider und Checkboxen; Spinner; Tooltips; Gruppen-Kontextmenü mit Pfeiltasten, Enter und Esc

**Tastatur (T0.2)**
- [ ] ⌘C in der Eintragsliste kopiert das Passwort, ⌘C im Suchfeld mit Auswahl kopiert den Text
- [ ] ⌘F ohne fokussiertes Feld und aus einem Feld der Tresor-Ansicht springt ins Suchfeld; ⌘F im Eintragseditor und in jedem anderen Overlay tut nichts, der Fokus bleibt im Feld
- [ ] Tab und ⇧Tab wie vorher; Enter im Master-Passwort-Feld entsperrt, Esc bricht ab

**Auto-Type, Starter, Abgleich**
- [ ] Hotkey ⌃⌥⌘V bei verstecktem Fenster; ⌘⇧T mit Countdown
- [ ] SAP-Start, die Temp-Datei wird danach gelöscht
- [ ] SharePoint-Sync und Konfliktansicht; iCloud-Beobachter (notify 8): Datei extern ändern, das Ereignis kommt an

**Listen, Fenster, Updates, VoiceOver**
- [ ] Vault-Switcher (⌘O); Liste mit etwa 3500 Einträgen scrollt flüssig
- [ ] ⌘W versteckt und sperrt, ein Dock-Klick holt das Fenster zurück; ⌘Q-Veto bei laufendem Speichern
- [ ] Die Update-Prüfung läuft (der Updater prüft die Mindestversion nicht)
- [ ] VoiceOver-Stichprobe: Beschriftungen unverändert oder besser

**Screenshots alt gegen neu**, jeweils hell und dunkel, gleiche Fenstergröße, gleicher Tresor, außerhalb des Repos abgelegt: Welcome, Unlock, Tresor-Browser, Editor, Einstellungen (alle Tabs), Konflikt, Switcher, About, Toast, Gruppen-Kontextmenü, Passwortfeld-Kontextmenü. Erlaubt ist nur die Verschiebung der Toast-Position durch K3. Jede andere Abweichung wird hier eingetragen und nicht still angepasst (4.4).

- Geprüft am: ______ auf macOS ______ und ______
- Abweichungen: ______
- Freigabe Migration (0.9.3): [ ] ja  [ ] nein
