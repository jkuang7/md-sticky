# Sticky

https://github.com/user-attachments/assets/7b61d4fa-8e2a-4b80-af09-37120dd7e8cb


## About

A sticky note app inspired by the "stickies" app that comes with MacOS. I found it frustrating that it did not support md syntax, and had a ton of unecessary formatting options.

This app is not tested on windows or linux, so there may be bugs, but I don't forsee any problems getting it to work on either.

## Features

- uses a markdown text editor, compatible with github-markdown syntax (`[ ]` to make checkboxes)
- customizable colors and a large default color palate
- minimal and unobtrusive sticky note appearance
- autosave, notes persist after quitting and reopening the app
- recoverable note closing with `Cmd + Shift + T` to reopen the last closed note
- easily move, navigate, resize, and set colors of notes with keyboard shortcuts
- local-first note storage with atomic snapshots and automatic corrupt-file recovery

## Installation

These instructions are for Apple Silicon Macs (M1, M2, M3, M4, or newer). You do not need Git or any programming experience. The first installation takes longer because your Mac must build the app from its source code.

### 1. Install the required tools

You only need to do this part once.

#### Install Node.js

1. Go to [nodejs.org](https://nodejs.org/).
2. Download the **LTS** version for macOS.
3. Open the downloaded `.pkg` file and keep clicking **Continue**, then click **Install**.

#### Open Terminal

Terminal is an app included with every Mac:

1. Press `Command + Space` to open Spotlight Search.
2. Type `Terminal`.
3. Press `Return`.

You will paste a few commands into this window. Paste one command at a time and press `Return` after each one. Do not type the `$` symbol sometimes shown in online examples.

#### Install Apple's build tools

Paste this into Terminal:

```sh
xcode-select --install
```

A window will appear. Click **Install**, accept the agreement, and wait for it to finish. If Terminal says the tools are already installed, continue to the next step.

#### Install Rust

Paste this into Terminal:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

When asked how to proceed, press `Return` to choose the default installation. When it finishes, quit Terminal and open it again so it can find Rust.

### 2. Download Sticky from GitHub

1. Return to the main page of this GitHub repository.
2. Click the green **Code** button near the top of the file list.
3. Click **Download ZIP**. You do not need to create a GitHub account or install Git.
4. Open your **Downloads** folder in Finder.
5. If you see a ZIP file, double-click it. You should now see a folder named something like `md-sticky-main`.

### 3. Install Sticky in Applications

1. Open Terminal again.
2. Type the following, then press the space bar once. Do not press `Return` yet:

   ```sh
   cd
   ```

3. Drag the `md-sticky-main` folder from Finder into the Terminal window. Terminal will add the folder's location after `cd `.
4. Press `Return`.
5. Paste this command and wait for it to finish:

   ```sh
   npm ci
   ```

6. Paste this command:

   ```sh
   npm run install:macos
   ```

The first build may take several minutes. Keep Terminal open while it runs. When it is finished, Sticky is installed as `/Applications/Sticky.app` and opens automatically.

Sticky is built locally and is not notarized by Apple. If macOS blocks it, try opening Sticky once, then go to **System Settings → Privacy & Security**, scroll down to **Security**, and click **Open Anyway** only if you trust this repository. Apple explains this warning in [Open apps safely on your Mac](https://support.apple.com/102445).

You can delete the downloaded `md-sticky-main` folder after Sticky is installed.

### Updating Sticky later

1. Quit Sticky by pressing `Command + Q`.
2. Download a fresh ZIP from GitHub by repeating step 2 above.
3. Repeat step 3 with the newly downloaded folder.

The installer safely replaces the copy in Applications and reopens it. Your notes are not replaced; they are stored separately at `~/Library/Application Support/local.jian.mdsticky/notes.json`.

### Developer commands

Run these from the downloaded project folder:

```sh
# Build the app without installing it
npm run app:build

# Check, build, and create an Apple Silicon ZIP in dist/
npm run package:macos
```

## Local fork status

This repository is a local-first macOS app with bundle identifier `local.jian.mdsticky`. Its updater is disabled, so local builds never contact or install releases from the upstream project's update feed. Release signing and notarization credentials are not configured.

The upstream package metadata declares MIT, but the upstream repository does not currently include a root license file. Clarify the license with the upstream owner before redistributing this fork publicly.

Notes are stored as Tiptap JSON in a versioned Rust-owned `notes.json`. Each save retains the last valid snapshot as `notes.previous.json`. If the current store is unreadable, exact damaged bytes are preserved under `backups/`, the previous valid snapshot is restored when possible, and the app adds a visible recovery notice note.

## App Specific Keyboard shortcuts

Default editor shortcuts (Cmd+X, Cmd+V, Cmd+C) are enabled

| Command                           | Action                                                                                              |
|-----------------------------------|-----------------------------------------------------------------------------------------------------|
| `Cmd + Q`                         | Quit the application                                                                                |
| `Cmd + W`                         | Close the focused note without deleting its saved data                                               |
| `Cmd + Shift + T`                 | Reopen the most recently closed note                                                                 |
| `Cmd + N`                         | Create new note                                                                                     |
| `Cmd + Shift + H`                 | Hide all note windows, or show them again if they are hidden                                        |
| `Cmd + /`                         | Focus next note                                                                                     |
| `Cmd + Alt + /`                   | Focus previous note                                                                                 |
| `Cmd + Alt + <Arrow Key>`         | Snap note (Move window in direction until it aligns with the nearest fully overlapping window edge) |
| `Cmd + Shift + Alt + <Arrow Key>` | Partially snap note (Move window in direction until it aligns with the nearest window edge)         |
| `Cmd + <Number>`                  | Set color of note                                                                                   |
| `Cmd + Shift + 0`                | Toggle a bullet list                                                                                |
| `Cmd + Shift + C`                | Check or uncheck the current task                                                                   |
| `Cmd + Shift + X`                | Delete all completed tasks in the focused note                                                      |
| `Cmd + Shift + S`                | Toggle strikethrough                                                                                |
| `Tab` / `Shift + Tab`            | Indent or outdent a list item                                                                       |
| `F1`                              | Show or hide the Keyboard Shortcuts window                                                          |
