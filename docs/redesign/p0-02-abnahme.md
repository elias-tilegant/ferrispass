# Abnahme P0.02: Fähigkeitsmatrix und Test-Fundament (Stufe 0)

Stand 15.10.2026, Zweig `p0-01-gpui-kit`, auf P0.01 (`07ac8e9`). Grundlage: Plan 30, F4 Zeile P0.02, Plan 20, 4.7 und 4.8. Dazu zwei Vorarbeiten, ohne die 0.9.3 nicht fertig wäre: das Sicherheitsupdate und die SDK-Prüfung in CI.

## 1 Commits

| Commit | Inhalt |
|---|---|
| `8368310` | rustls 0.23.46, quinn-proto 0.11.18 |
| `e5add59` | CI prüft das macOS-SDK gegen die Mindestversion 26 |
| `381c46e` | T0.3: `tests/architecture.rs` |
| `439bbe4` | T0.4: proptest und zwei Property-Tests |
| `0cf0bad` | `docs/gpui-matrix.md` |
| `48961e8` | Befunde aus dem Codex-Gate im Architekturtest geschlossen |

## 2 Gates

| Gate | Ergebnis |
|---|---|
| `cargo fmt --all --check` | grün |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | grün (nur die bekannte Future-Incompat-Notiz zu `block` 0.1.6) |
| `cargo test --locked` | grün, kein neuer `#[ignore]` |
| `cargo build --locked --bin ferrispass-cli` | grün |
| `cargo audit` | **grün**, vorher 7 Schwachstellen (Abschnitt 3) |
| Gedankenstrich-Scan über `git ls-files` und über die Commit-Messages `master..HEAD` | leer |

## 3 Sicherheitsupdate

`cargo audit` vorher: 7 Schwachstellen, alle schon auf `master`.
- rustls 0.23.45: RUSTSEC-2026-0346 bis -0349 (Fix 0.23.46). rustls trägt jeden HTTPS-Aufruf der App.
- quinn-proto 0.11.17: RUSTSEC-2026-0338, -0340, -0343 (Fix 0.11.18). Steht über reqwest und gpui-pre-reqwest im Lock, `cargo tree --target all -e all --all-features` findet aber keinen Pfad in einen Build.

Nachher: keine Schwachstelle. Fünf Warnungen "unmaintained" bleiben stehen, wie gefordert nur notiert: instant 0.1.13 (RUSTSEC-2024-0384), paste 1.0.15 (RUSTSEC-2024-0436), rustls-pemfile 2.2.0 (RUSTSEC-2025-0134), rustybuzz 0.20.1 (RUSTSEC-2026-0206), ttf-parser 0.25.1 (RUSTSEC-2026-0192).

**Lock:** Im Commit ändern sich nur die beiden Pakete, je Version und Prüfsumme, gpui-base und gpui-component-macros bleiben auf 0.7.1, kein gpui-fast. `cargo update -p ... --precise` hätte zusätzlich sieben Kanten mit Versionsbereichen umgehängt (`windows-sys` bei errno, rustix 0.38 und 1.1, quinn-udp, winapi-util und tempfile, `getrandom` bei tempfile von 0.4.3 auf 0.3.4), auf andere schon gelockte Versionen. Diese Kanten wählen `master`, P0.01 und jeder `cargo update` verschieden. Die beiden Anhebungen brauchen das nicht. Übernommen sind deshalb nur Version und Prüfsumme aus cargos Ergebnis, und `cargo metadata --locked` bestätigt den Lock.

## 4 SDK-Prüfung in CI

`ci.yml` prüft als ersten Schritt nach dem Checkout, ob `xcrun --sdk macosx --show-sdk-version` mindestens die Mindestversion ist. Die Mindestversion liest der Schritt aus `MIN_MACOS` in `scripts/build-mac.sh`, sie steht also an einer Stelle. Weil `release.yml` auf demselben Image (`macos-latest`) läuft, zeigt jeder PR, ob der Release-Runner für macOS 26 bauen kann. Lokal geprüft: SDK 27.0 gegen 26.0 grün; SDK 15.5, ein leerer Wert, ein scheiterndes `xcrun`, ein fehlendes `MIN_MACOS` und SDK 27.0 gegen 28 jeweils rot. Ein leerer Wert ließe `[ -lt ]` mit Exit 2 enden, und ein `if` läse das als bestanden. Deshalb prüft der Schritt beide Werte vorher auf Zahlen. Den Beleg auf dem Runner liefert erst der erste CI-Lauf nach dem Push.

## 5 Fähigkeitsmatrix (4.7)

[`docs/gpui-matrix.md`](../gpui-matrix.md) belegt alle Zeilen aus 4.7 und die Stellen, auf die die Spikes zeigen, mit Datei und Zeile in Zed 1870e26 und gpui-pre 0.3.8. Alle zitierten Zeilen sind per Skript gegen die Quellen gelesen. Die Fehlanzeigen (kein `sharingType`, kein eigener `PromptBuilder` in Kit, keine Reduce-Motion-Lesung in gpui-pre-macos) sind per grep geprüft.

Ergebnis: Für FerrisPass hat sich zwischen den beiden Ständen nichts geändert, was zählt. Was die Spikes daraus mitnehmen:
- `canBecomeKeyWindow` ist weiter fest `YES`, und `activate` macht jedes Fenster Key. SP03 bleibt nötig.
- `application:openURLs:` verwirft URLs, solange kein Callback gesetzt ist. SP06 muss den Kaltstart prüfen, `on_open_urls` gehört an den `Application`-Builder vor `run`.
- GPUI kennt `sharingType` nicht, SP12 geht über `HasWindowHandle`.
- GPUI Kit liest "Bewegung reduzieren" einmal bei `init`, folgt einer Änderung auf macOS aber nicht. Antwort für DS-0 in P0.07.
- Die Mindestversion von gpui-pre liegt bei 10.15.7 (Shader), weit unter 26. Es braucht keine neue Entscheidung (4.5).

**Laufversuche:** keine ausgeführt. Jeder bräuchte die echte AppKit-Plattform oder ein Fenster auf Elias' Arbeits-Mac. Alle Zeilen stehen auf "ausstehend, Spike-Lauf". Der Abschnitt Spike-Ergebnisse (SP01 bis SP14 mit P-Paket) ist angelegt und leer.

**Headless-UI-Tests auf dem Runner (Frage aus F4):** Laut Quelltext brauchen die Tests von FerrisPass weder Metal noch einen Fensterserver. `#[gpui::test]` läuft auf `TestPlatform` mit `NoopTextSystem` und ohne Renderer, Metal braucht erst `HeadlessAppContext` mit echtem Renderer. Den Beleg auf `macos-latest` liefert der erste CI-Lauf.

## 6 T0.3 Architekturtest

`tests/architecture.rs` führt die Ausnahmeliste aus Plan 20, 2.10 Regel 4, nach Datei und Item statt nach Zeile:

| Datei | Item | Zeile heute |
|---|---|---|
| `src/session_lock.rs` | ganze Datei | 8 |
| `src/update/macos_installer.rs` | ganze Datei | 7 |
| `src/biometric/macos.rs` | ganze Datei | 19 |
| `src/launch/tempfile.rs` | `fn process_uid` | 182 |
| `src/keepass/document.rs` | `fn publish_new_no_clobber` | 1695 |
| `src/keepass/document.rs` | `mod macos_full_sync` | 1740 |
| `src/sync/icloud.rs` | `mod platform` | 268 |

Neue Module kommen in `PLATFORM_MODULES`, nur unter `src/platform/`, mit `//!`-Kopf. Gelesen wird jede `.rs`-Datei im Repo außer `target/` und `.git/`. Der Test scheitert:
- an jedem Attribut, das den Lint lockert und nicht in der Liste steht (`allow`, `expect`, `warn`, auch in `cfg_attr` und als Raw-Identifier), und an Listeneinträgen ohne Attribut
- an jedem `unsafe` außerhalb der gelisteten Items: Blöcke, Funktionen, `unsafe impl`, `unsafe extern`, `no_mangle`, `export_name`, `link_section`, `allow_internal_unsafe`, `global_asm` in jeder Schreibweise, auch in Makro-Körpern. So hält die Regel auch dann, wenn jemand den Lint senkt.
- an `unsafe_code` außerhalb eines Attributs, etwa als Makro-Argument, aus dem ein Makro ein `allow` baut
- an einer Lint-Stufe unter `deny` in `Cargo.toml`. Der Eintrag wird als TOML gelesen (Kommentare, Anführungszeichen, `unsafe-code` als zweite Schreibweise, genau ein Eintrag).
- an Lint-Flags außerhalb von `Cargo.toml`: `unsafe_code`, `cap-lints`, `--config` oder `include` in `.cargo/config.toml`, `rustflags`, `cap-lints` oder der Lint in den CI-Workflows, jeweils nach Auflösen von Escapes
- an Code, den der Scan nicht liest: `include!` in jeder Schreibweise, ein `#[path]` oder ein Build-Target in `Cargo.toml`, das keine `.rs`-Datei im Repo ist

Der Scan arbeitet auf Tokens. Kommentare, Doc-Kommentare, Strings, Raw-Strings und Zeichenliterale sind nie Befunde. Ein Quelltext, den der Lexer nicht sauber schließt, ist ein Fehler und kein Bestehen. Eigene Tests decken diese Formen ab.

**Mutationsproben** (jeweils zurückgenommen, Baum danach sauber):
1. `#[allow(unsafe_code)]` mit `unsafe`-Block in `src/app/recents.rs` (Unterordner, nicht gelistet): rot, mit Datei, Zeile und `fn architecture_probe`.
2. Neues Opt-in an einem anderen Item in der gelisteten Datei `src/keepass/document.rs`: rot.
3. Lint in `Cargo.toml` auf `allow` und ein `unsafe`-Block ohne Attribut in `src/ui/widgets/secret_input.rs`: beide Tests rot, mit Lint-Stufe und Fundstelle.

**Grenzen:** Unsafe Code, den ein Makro einer fremden Crate erzeugt, ohne dass `unsafe` im Quelltext von FerrisPass steht, sieht der Scan nicht. rustc meldet `unsafe_code` für solche Expansionen auch nicht, diese Crates prüft die Lieferkette. `static mut` lassen Scan und rustc als Deklaration durch, jeder Zugriff braucht aber `unsafe`. Flags aus der Umgebung oder aus einer Cargo-Konfiguration oberhalb des Repos erreicht der Test nicht; die CI setzt keine, und die Workflow-Prüfung hält das fest.

## 7 T0.4 Property-Tests

Werkzeug ist proptest (Elias, 14.10.2026). Es steht jetzt als Dev-Abhängigkeit in `Cargo.toml`, mit zwei echten Property-Tests in `src/keepass/password_gen.rs`:
- `generate` über Längen von 0 bis 200 und alle 16 Klassen-Kombinationen: Länge auf 4 bis 128 begrenzt, nur Zeichen aus aktiven Klassen (ohne aktive Klasse Kleinbuchstaben), jede aktive Klasse kommt vor.
- `estimate_bits` und die Stärke-Stufe fallen nie, wenn das Passwort länger wird.

Der Lock bekommt nur die Kante von FerrisPass, denn gpui-pre/test-support zieht dieselbe proptest-Version schon.

**Mutationsprobe:** Ohne `required.push(SYMBOLS)` wurde in der ersten Fassung nur der alte Beispiel-Test rot, der Property-Test nicht. Bei gleichverteilten Längen bis 200 kamen zu selten kurze Passwörter vor, und nur bei denen fehlt eine Klasse. Jetzt ist die Hälfte der Fälle höchstens 8 Zeichen lang. Damit wird der Test rot und schrumpft auf Länge 4. Die Regressionsdatei dieser Probe ist gelöscht. Echte Fehlschläge legt proptest unter `proptest-regressions/` ab, die gehören dann ins Repo.

## 8 Codex-Gate

Ein Lauf mit `gpt-5.6-sol`, `model_reasoning_effort=xhigh`, read-only, ohne cargo. Frage: Kann `tests/architecture.rs` neues `unsafe` außerhalb der Liste übersehen, und stimmen fünf zufällige Belege der Matrix?

**Matrix:** Die fünf gezogenen Zeilen (PopUp-Spaces, `on_open_urls`, `observe_window_activation`, `cx.hide`, `reduce_motion`) stimmen in allen Zeilenangaben, und der Code trägt die Aussagen, auch die Fehlanzeige in gpui-pre-macos.

**Architekturtest:** vier schwere Befunde, alle berechtigt und in `48961e8` behoben. Jeder wurde vorher am alten Test reproduziert (grün trotz Lücke), danach ist der neue Test rot:
1. Quellen außerhalb der gescannten Ordner (`#[path = "x.inc"]`, ein Target wie `tools/helper.rs`). Jetzt wird das ganze Repo gelesen, `#[path]` und Targets müssen `.rs` im Repo sein. Probe: `tools/helper.rs` mit `allow` und `unsafe`.
2. Schreibweisen von Makros: `r#global_asm!`, `use core::arch::global_asm as ga`, `r#include!`. Jetzt nach Namen in jeder Schreibweise, `include` auch als Pfad, Umbenennung oder Makro-Argument. Belegt durch Unit-Tests, im Baum würde rustc `global_asm` schon ablehnen.
3. Die Lint-Stufe wurde per Teilstring gefunden: `unsafe_code = "allow" # was "deny"` bestand. Jetzt als TOML gelesen. Probe: genau diese Zeile in `Cargo.toml`.
4. `.cargo/config.toml` wurde als Rohtext geprüft, `"--cap\u002dlints=allow"` fiel nicht auf. Jetzt mit aufgelösten Escapes, dazu die Workflows. Probe: `RUSTFLAGS: "--cap\u002dlints=allow"` in `ci.yml`.

Kleinere Befunde: `#[allow_internal_unsafe]` zählt jetzt als unsafe, eine Shebang-Zeile ist kein Code. Bewusst nicht geändert: `static mut` (siehe Grenzen), Makros fremder Crates (Lieferkette) und der Fehlalarm bei `#[cfg(no_mangle)]`, der in die sichere Richtung fällt.

Hinweis: Codex hat nach dem Lauf über das MCP-Werkzeug von sich aus einen Eintrag in Hindsight geschrieben (Scannerlücken und Stichprobe). Dateien hat der Lauf nicht verändert.

## 9 Offen

- **CI-Lauf:** SDK-Schritt und Headless-Tests auf `macos-latest` sind erst nach dem Push belegt. Kein Push in diesem Lauf.
- **Laufversuche der Matrix:** in den Spikes, Ergebnisse in `docs/gpui-matrix.md`.
- **Unmaintained-Warnungen** im Audit (Abschnitt 3): nur notiert.
- **Aus P0.01 weiter offen:** Elias' OK zur Dev-Abhängigkeit (`gpui` und `gpui-component` mit `test-support` statt `gpui-kit`), das manuelle Protokoll Teil 0 und die Freigabe der Migration.
