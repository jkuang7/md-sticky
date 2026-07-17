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
- Every native exit request is blocked by an explicit quit state machine until each note webview acknowledges a successful save; only the coordinator's final exit is permitted.
- Close requests target only the focused note webview; application-wide emits are reserved for coordinated save-on-quit.
- Closing a note archives it with a durable timestamp before closing its window. Archived notes remain in `notes.json`, are skipped at launch, are never automatically purged, and can be restored most-recent-first from the application menu.
- Duplicate launches and macOS Dock reopens focus existing notes, creating one only when the running process has no windows.
- The saved autostart preference is reconciled against the launch agent on every release launch. Development builds disable login-item registration.
- The updater is intentionally absent. Local bundles use ad-hoc signing; notarization and public distribution require a separate release decision.
