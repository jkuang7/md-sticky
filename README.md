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

### macOS

This private local fork builds an ad-hoc-signed `.app` bundle. It is not notarized or intended for public distribution.

## Local fork status

This repository is a private macOS app with bundle identifier `local.jian.mdsticky`. Its updater is disabled, so local builds never contact or install releases from the upstream project's update feed. Notarization, release credentials, and public distribution are intentionally out of scope.

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
| `Cmd + /`                         | Focus next note                                                                                     |
| `Cmd + Alt + /`                   | Focus previous note                                                                                 |
| `Cmd + F`                         | Resize note to text                                                                                 |
| `Cmd + Alt + <Arrow Key>`         | Snap note (Move window in direction until it aligns with the nearest fully overlapping window edge) |
| `Cmd + Shift + Alt + <Arrow Key>` | Partially snap note (Move window in direction until it aligns with the nearest window edge)         |
| `Cmd + <Number>`                  | Set color of note                                                                                   |
