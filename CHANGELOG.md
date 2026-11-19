# Changelog

## 1.2.1

### Safer
- Undo only moves files a job made to the Recycle Bin; an editor window can only save its edits.
- Quick Look reads only the files it was opened on.
- Help links open only the exact pages they are meant for.
- FFmpeg reads local files only (a crafted playlist can't make it fetch URLs).
- Linux and macOS: Rotae's scratch folder is private to the user instead of a shared one in /tmp.
- Comparing two files in VS Code skips names with quotes (Linux), add-on downloads are capped at 2 GB.

### Fixed
- A recipe with a custom command no longer deletes files that only appeared in the folder meanwhile.
- An unreadable Activity list or findings file is kept as `*.broken.json` instead of being replaced.
- Watched folders never process Rotae's own results again after a long time running.
- CSV: a quote inside a value (`12" pipe`) no longer swallows the rest of the row.
- Redaction covers the last partly covered pixel column and row.
- Quick Look shows details for camera RAW, HEIC and other non-text files instead of garbage text.
- Pasting into a text box in the hub no longer also pastes files.
- An update offer hidden by a job's card comes back when the card goes.
- Failures that happen while the hub is hidden keep the Activity badge lit.
- Vietnamese: the actions search box and a few tooltips that stayed in English are translated.
- A desktop without a tray no longer stops Rotae from starting.

### Accessibility
- Screen readers get the current hub view, the action list as a list box, named icon buttons and
  announced errors.

### For developers
- `npm run check` runs every check; front-end unit and UI tests, ESLint and `cargo fmt --check` in CI.
- `ROTAE_DATA_DIR` runs a separate development copy.

## 1.2.0
- Rotae for Mac (Apple silicon, macOS 14+).

## 1.1.1
- Clearer help when Ubuntu blocks the pentest sandbox; text to PDF fixes on Linux.

## 1.1.0
- Linux version and Pentest user mode.

## 1.0.0
- First release (Windows).
