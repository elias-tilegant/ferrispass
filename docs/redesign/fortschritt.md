# Fortschritt UI/UX-Neubau

Ist-Werte je Paket. Elias trägt die Stunden seiner eigenen Zeit ein (Review, Tests am Gerät, Abnahme), die Agenten-Laufzeit ist die Wanduhrzeit der Agentenläufe. Aus P0.01 bis P0.04 entsteht der erste Kalender (Fahrplan F1.5).

| Paket | Datum | Elias-Stunden | Agenten-Laufzeit | Notiz |
|---|---|---|---|---|
| P0.01 | 14.10.2026 | | 0,6 h (nur Implementierung; Lieferkette, Review, Codex-Gate und Nacharbeit nicht eingerechnet) | Migration auf GPUI Kit (gpui-component 0.7.1, gpui-pre 0.3.8), notify 8, K1 bis K9, K11 und K12 (K10, das Memory, ist offen), T0.1 und T0.2, Lizenztexte, Lieferketten-Skript, Codex-Gate. `cargo audit` ist rot durch rustls 0.23.45 und quinn-proto 0.11.17, wie schon auf `master`. Abnahme: [p0-01-abnahme.md](p0-01-abnahme.md) |
