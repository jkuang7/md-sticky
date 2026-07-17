# Sticky Architecture

## Note ownership

- Rust is the sole owner of durable note state. `NoteRepository` serializes every read-modify-write operation behind one mutex and atomically replaces `notes.json` only after the temporary file has been flushed.
- Each note has a stable UUID and stores Tiptap JSON, color, logical position, expanded dimensions, collapsed state, and pin state. Window labels derive from the UUID and are not persistence identities themselves.
- The webview owns only the live Tiptap editor instance. It sends complete document snapshots to Rust on debounced edits and flushes them at blur, fold, close, and application-quit boundaries.
- Clipboard interoperability uses Tiptap's schema-backed defaults: copy provides HTML and plain-text representations, while paste retains supported semantic structure and discards unsupported markup.

## Snapshots and recovery

- Before each normal replacement, the last valid `notes.json` is atomically retained as `notes.previous.json`.
- If `notes.json` is unreadable, its exact bytes are preserved under `backups/`. A valid previous snapshot is restored when available; an unreadable previous snapshot is also preserved exactly.
- Recovery adds a visible notice note. With no valid snapshot, that notice is the only note in a fresh store.
- Only a brand-new store receives an initial empty note. A valid empty store represents intentional user state and remains empty across restarts.

## Window lifecycle

- Expanded dimensions remain canonical while a note is collapsed. Rust performs fold and unfold resizing, and clamps an expanded note to the display containing its collapsed header.
- macOS title-bar drags are anchored by Rust to the target native window and current cursor position; frontend code only initiates the drag gesture.
- New notes use the focused note as their anchor and persist the closest edge-aligned position that fits within its monitor work area without overlapping another note.
- An optional linked-stack ID order is the sole durable grouping state. Tapping any chain button recaptures every note's order from its current vertical position, uses the topmost open note as the leader and anchor, and resets open members to one left edge at fixed title-bar intervals (24px title bar plus a 12px gap), so expanded note bodies may overlap without pushing later headers down. The note being viewed is frontmost within its pin level: clicking or unfolding a note raises its full body above its peers, and folding it back to the title bar reveals the notes beneath it. Pinned windows still remain above unpinned windows. During a macOS title-bar drag, every other open member is temporarily attached as a native child behind the active note so AppKit moves the group synchronously; cleanup refocuses the active note so a click without movement cannot expose a window underneath. Other platforms apply the leader's drag delta. Unlinking remains available from the application menu and leaves each note's saved geometry untouched.
- Keyboard shortcut help uses a dedicated non-note window toggled by Help → Keyboard Shortcuts or F1. It is excluded from note ordering and coordinated note-save shutdown.
- Cmd+Shift+H toggles the visibility of all open note windows without changing their durable note state; restoring the notes returns focus to the previously focused note when it is still open.
- Beginner installation is owned by the root `install.sh`: after Git obtains the repository, it checks Apple Silicon and Apple's compiler tools, installs checksum-verified official Node and Rust toolchains under the ignored `.tools/` directory, installs locked project dependencies, and delegates safe app replacement to `scripts/install-macos.sh`. It does not modify a user's existing Node, Rust, or shell configuration.
- Every native exit request is blocked by an explicit quit state machine until each note webview acknowledges a successful save; only the coordinator's final exit is permitted.
- Close requests target only the focused note webview; application-wide emits are reserved for coordinated save-on-quit.
- Closing a note archives it with a durable timestamp before closing its window. Archived notes remain in `notes.json`, are skipped at launch, are never automatically purged, and can be restored most-recent-first from the application menu.
- Duplicate launches and macOS Dock reopens focus existing notes, creating one only when the running process has no windows.
- The saved autostart preference is reconciled against the launch agent on every release launch. Development builds disable login-item registration.
- The updater is intentionally absent. Local bundles use ad-hoc signing; notarization and public distribution require a separate release decision.
