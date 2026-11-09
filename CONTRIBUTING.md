# Contributing to openMapper

Thank you for helping. A few rules keep the project easy to work on.

## Issues

* Write in English; one problem per issue; use the latest release.
* For a bug say what you did, what you expected and what happened, and attach the map's `global.cfg`
  path and the end of the log if the program stopped. A screenshot helps.
* Security problems are reported privately, see [SECURITY.md](SECURITY.md).

## Pull requests

* One change per pull request, based on the latest `main`.
* `cargo build --release` and `cargo test --release` pass (CI runs them).
* Keep large files out of the repository: link screenshots and videos in the pull request and keep
  test data small.
* Write code that reads like the code around it: its naming, its comment density, its idiom. The
  interface texts come in English and Russian (`tr("English", "Русский")`) and, with every change
  that adds or rewords one, in every other language of the interface: see [Translations](#translations).
* New code goes into new files where it can: shared files (`src/ui.rs`, `src/editor.rs`,
  `src/app.rs`, `src/main.rs`) only get small hooks. See [docs/ROADMAP.md](docs/ROADMAP.md).
* Commit messages say what changed for the user, in plain words.

## Building

```bash
cargo run --release
```

Linux needs `libgtk-3-dev`, `libxkbcommon-dev` and `libwayland-dev`. The logo and icons are made by
`python3 tools/make-icons.py` (Pillow, fontTools, skia-pathops).

## Translations

The interface speaks the twenty-eight languages of openOMSI plus Serbian Latin (View > Language). English and Russian
are written where a text is used: `tr("Save", "Сохранить")`. Every other language is a table found
by the English text, `locales/<code>.tsv`: the English, a tab, the translation (`\n`, `\t` and `\\`
stand for a newline, a tab and a backslash). A text without an entry is shown in English.

**The rule: every change that brings an interface text brings its translation into every
`locales/*.tsv` file, in the same pull request.** A new menu, a button, a hint, a toast, an error
message, a dialog: all of it, in all languages, not only English and Russian. A reworded English
text is a new text. `python3 tools/i18n/check.py` lists what is missing (CI runs it and fails on a
text without a translation, or whose `{}` / `%` placeholders differ from the English one);
`python3 tools/i18n/check.py --todo de` prints the texts `de` still lacks.

* Keep the placeholders (`{}`, `{n}`, `%d`), keys, file names, units and product names as they are.
* Follow the wording of openOMSI's own table (`crates/omsi-app/locales/app.yml` there) for the terms both
  programs use: tile, spline, junction, timetable.
* A language that is not in the list needs a row in `src/i18n.rs` (`LANGUAGES`, `Lang`), its table, and,
  for a script the interface font lacks, a font of the system's in `src/sysfont.rs`.
