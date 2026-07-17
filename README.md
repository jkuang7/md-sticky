# StickyMD

A minimal, local-first Markdown sticky-note app for Apple Silicon Macs. The installed macOS app is named **Sticky**.

https://github.com/user-attachments/assets/7b61d4fa-8e2a-4b80-af09-37120dd7e8cb

Sticky is a personal fork of the original macOS Stickies-inspired project. This fork focuses on reliable local storage, keyboard-driven note management, linked note stacks, and a straightforward source-based installer.

## Features

- Markdown editing with task lists and GitHub-style syntax
- Autosave with notes restored after quitting or restarting the Mac
- Local-only note storage with atomic snapshots and corrupt-file recovery
- Custom note colors, pinning, folding, snapping, and linked vertical stacks
- Recoverable note closing: reopen the last closed note with `Command + Shift + T`
- Hide or restore all notes with `Command + Shift + H`
- Keyboard shortcut reference available inside the app with `F1`

## Install on a Mac

Sticky supports Apple Silicon Macs (M1 or newer) and requires Git. Open **Terminal** and run:

```sh
git clone https://github.com/jkuang7/StickyMD.git
cd StickyMD
./install.sh
```

The script installs its build tools inside the repository, builds Sticky, installs it in Applications, and opens it. Keep Terminal open; the first run may take several minutes. If macOS asks to install developer tools, accept the installation and rerun the command that stopped.

### If macOS blocks the first launch

Sticky is built locally and is not notarized by Apple. If macOS blocks it:

1. Try to open Sticky once.
2. Open **System Settings → Privacy & Security**.
3. Scroll down to **Security**.
4. Click **Open Anyway** only if you trust this repository.

See Apple's [Open apps safely on your Mac](https://support.apple.com/102445) guidance for more information.

## Update Sticky

If you used the installation commands above, the repository is in your home folder. Open Terminal and run:

```sh
cd ~/StickyMD
git pull --ff-only
./install.sh
```

The installer rebuilds Sticky, safely replaces the app, and reopens it. Updating the app does not replace your notes.

## Notes and privacy

Sticky has no cloud sync, analytics, release updater, or account system. Notes stay on the Mac at:

```text
~/Library/Application Support/local.jian.mdsticky/notes.json
```

The last valid snapshot is retained as `notes.previous.json`. If the current file becomes unreadable, Sticky preserves the damaged bytes under `backups/` and restores the previous snapshot when possible.

Closing a note with `Command + W` archives it instead of immediately deleting its saved data. Press `Command + Shift + T` to reopen the most recently closed note.

## Uninstall

1. Quit Sticky with `Command + Q`.
2. Move `/Applications/Sticky.app` to the Trash.
3. Delete the `StickyMD` repository folder if you no longer need it for updates.

Your notes remain in the Application Support folder shown above unless you delete that folder separately.

## Keyboard shortcuts

Press `F1` inside Sticky to open the built-in shortcut reference.

| Shortcut | Action |
|---|---|
| `Command + Q` | Quit Sticky |
| `Command + W` | Close the focused note while retaining its saved data |
| `Command + Shift + T` | Reopen the most recently closed note |
| `Command + N` | Create a note |
| `Command + Shift + H` | Hide or restore all notes |
| `Command + /` | Focus the next note |
| `Command + Option + /` | Focus the previous note |
| `Command + Option + Arrow` | Snap the note to the next overlapping edge |
| `Command + Shift + Option + Arrow` | Snap the note to any nearby edge |
| `Command + 1–7` | Set the note color |
| `Command + Shift + 0` | Toggle a bullet list |
| `Command + Shift + C` | Check or uncheck the current task |
| `Command + Shift + X` | Delete completed tasks |
| `Command + Shift + S` | Toggle strikethrough |
| `Tab` / `Shift + Tab` | Indent or outdent a list item |
| `F1` | Show or hide the keyboard shortcut reference |

Standard editing shortcuts such as copy, cut, paste, undo, and redo also work.

## Development

Run `./install.sh` once to create the project-local Node.js and Rust toolchains. In each new Terminal session, enter the repository and expose those tools for development:

```sh
cd ~/StickyMD
export CARGO_HOME="$PWD/.tools/cargo"
export RUSTUP_HOME="$PWD/.tools/rustup"
export PATH="$PWD/.tools/node/bin:$CARGO_HOME/bin:$PATH"
```

Useful commands:

```sh
npm run dev
npm run check
npm run app:build
npm run package:macos
npm run install:macos
```

`npm run package:macos` creates `dist/Sticky-macOS-arm64.zip`. `npm run install:macos` checks, rebuilds, safely replaces the app in Applications, and reopens it.

Architecture and persistence details are documented in [PLOT.md](PLOT.md).

## Project status and licensing

This fork uses bundle identifier `local.jian.mdsticky`. Builds are ad-hoc signed and are not notarized for public distribution.

The inherited package metadata declares MIT, but the upstream repository does not include a root license file. Confirm the licensing terms with the upstream owner before redistributing compiled builds or source beyond personal testing.
