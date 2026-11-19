# Contributing

Thanks for helping. Bug reports and ideas go to
[issues](https://github.com/nhaajtt/rotae/issues); the app's Report button fills in the versions.

## Before a pull request

Run `npm run check`. It is what CI runs (lint, front-end unit and UI tests, the web build,
`cargo fmt --check`, Clippy with warnings as errors, engine tests). See README › Build from source.

Keep pull requests to one change. Write commit messages in plain words about what changes for
the user.

## Adding an action

An action is something the wheel or the hub can do with files. It needs three pieces:

1. **The menu entry** in `src/menu.js`, in the menu of the file kind it applies to
   (`imageMenu`, `pdfMenu`, …). The entry's `action` is the JSON the backend receives, for example
   `{ op: 'resize', max: 1600 }`. Fields the user must type (a password, a size) go in `ask`.
2. **The action** in `src-tauri/src/engines/mod.rs` (`enum Action`, `op` in snake_case), and the
   code in the engine of that file kind, e.g. `engines/image.rs`. The engine writes its result next
   to the original with `naming::out_path`, which never overwrites a file.
3. **Tests.** `npm run test:update` records the new menu in `tests/unit/golden/`; review that diff.
   The Rust test `every_action_the_ui_sends_parses` then checks the backend accepts the new JSON.
   Add an engine test in `engines/tests.rs` that runs the action on a generated sample.

If the label is new, add its Vietnamese text to `src/i18n.js`.

## Conventions

- Rust: `cargo fmt` (see `rustfmt.toml`), Clippy clean. Errors are written for the user, and
  `src/fixes.js` turns known ones into a "why / how to fix" card.
- JavaScript: plain modules, no framework; build DOM with `h()` from `src/ui.js`, never `innerHTML`
  with file names or file contents.
- Windows (webviews) only get the backend commands they need: `src-tauri/capabilities/`.
- Comments say why, not what.
