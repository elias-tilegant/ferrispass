# Redesign progress

Time spent per step. "My time" covers review, testing on a real Mac and sign-off. "Agent time" is how long the coding agents ran. Once P0.01 to P0.04 are done, I'll use their numbers to schedule the rest of the redesign.

| Step | Date | My time | Agent time | Notes |
|---|---|---|---|---|
| P0.01 | 2026-10-14 | | 0.6 h (implementation only, without supply-chain work, review and follow-up fixes) | Moved to GPUI Kit (gpui-component 0.7.1 on gpui-pre 0.3.8), bumped `notify` to 8, raised the minimum to macOS 26, added tests for protected fields and key bindings, bundled the license texts, added the supply-chain script. `cargo audit` failed on rustls 0.23.45 and quinn-proto 0.11.17, which `master` already had. Fixed in `8368310`. See [p0-01-gpui-kit.md](p0-01-gpui-kit.md). |
| P0.02 | 2026-10-15 | | 1.1 h (including review and fixes) | Wrote the GPUI capability matrix in `docs/gpui-matrix.md` from reading the source. Runtime checks are left to the spikes. Added the architecture test for `unsafe` and property tests for the password generator. Also bumped rustls to 0.23.46 and quinn-proto to 0.11.18 (`cargo audit` passes) and added the SDK check to CI, both needed for 0.9.3. See [p0-02-matrix-and-tests.md](p0-02-matrix-and-tests.md). |
| P1a.04 | 2026-10-28 | | 0.6 h (including review and fixes) | Added `src/app/kinds`, which works out from an entry's custom fields, title, tags and URL whether it is an SAP system (with SID, client, role and line), remote access or a web login. Display only: nothing is written, and nothing in the app shows it yet. See [p1a-04-entry-kinds.md](p1a-04-entry-kinds.md). |
