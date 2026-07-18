<script lang="ts">
  import {
    mdiClose,
    mdiLinkVariant,
    mdiPalette,
    mdiPin,
    mdiPinOff,
  } from "@mdi/js";
  import { invoke } from "@tauri-apps/api/core";
  import { webviewWindow } from "@tauri-apps/api";
  import { onDestroy, onMount } from "svelte";

  import Editor from "$lib/Editor.svelte";
  import Icon from "$lib/Icon.svelte";
  import ShortcutsHelp from "$lib/ShortcutsHelp.svelte";

  interface StickyInit {
    always_on_top?: boolean;
    collapsed?: boolean;
  }

  const colors = [
    "#fff9b1",
    "#81b7dd",
    "#65a65b",
    "#aad2ca",
    "#98c260",
    "#e1a1b1",
    "#b98cb3",
  ];
  const appWindow = webviewWindow.getCurrentWebviewWindow();
  const shortcutsWindow = Boolean(
    (window as typeof window & { __SHORTCUTS__?: boolean }).__SHORTCUTS__,
  );

  let editor = $state<Editor>();
  let colorMenuOpen = $state(false);
  let titlebarHovered = $state(false);
  let alwaysOnTop = $state(false);
  let collapsed = $state(false);
  let stackBusy = $state(false);
  let noteTitle = $state("Empty Note");
  let moveTimer: number | undefined;
  const unlisteners: Array<() => void> = [];

  async function toggleAlwaysOnTop() {
    await editor?.flushSave();
    alwaysOnTop = !alwaysOnTop;
    await invoke("set_note_always_on_top", { alwaysOnTop });
  }

  async function linkAllNotes() {
    if (stackBusy) return;
    stackBusy = true;
    try {
      await editor?.flushSave();
      await invoke("link_all_notes_to_current_note");
    } finally {
      stackBusy = false;
    }
  }

  async function closeNote() {
    await editor?.flushSave();
    await invoke("close_window");
  }

  async function toggleCollapsed() {
    await editor?.flushSave();
    const next = !collapsed;
    await invoke("set_collapsed", { collapsed: next });
    collapsed = next;
    colorMenuOpen = false;
    if (!collapsed) requestAnimationFrame(() => editor?.focus());
  }

  function toggleColorMenu() {
    colorMenuOpen = !colorMenuOpen;
  }

  async function setColor(color: string) {
    document.body.style.backgroundColor = color;
    colorMenuOpen = false;
    await editor?.flushSave();
  }

  function saveGeometryDebounced() {
    if (moveTimer !== undefined) window.clearTimeout(moveTimer);
    moveTimer = window.setTimeout(() => void editor?.flushSave(), 150);
  }

  function finishWindowDrag() {
    void invoke("finish_window_drag");
  }

  function createNoteWithControlN(event: KeyboardEvent) {
    if (
      event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !event.shiftKey &&
      event.key.toLowerCase() === "n"
    ) {
      event.preventDefault();
      event.stopPropagation();
      void invoke("create_note");
    }
  }

  onMount(async () => {
    if (shortcutsWindow) return;
    const init = (window as typeof window & { __STICKY_INIT__?: StickyInit })
      .__STICKY_INIT__;
    alwaysOnTop = init?.always_on_top ?? false;
    collapsed = init?.collapsed ?? false;

    if (!init) document.body.classList.add("focused");
    window.addEventListener("keydown", createNoteWithControlN, true);
    window.addEventListener("mouseup", finishWindowDrag, true);

    unlisteners.push(
      await appWindow.listen("tauri://focus", async () => {
        await invoke("bring_all_to_front");
        titlebarHovered = true;
        document.body.classList.add("focused");
      }),
      await appWindow.listen("tauri://blur", async () => {
        titlebarHovered = false;
        document.body.classList.remove("focused");
        editor?.removeSelection();
        await editor?.flushSave();
      }),
      await appWindow.listen<number>("set_color", async (event) => {
        await setColor(colors[event.payload]);
      }),
      await appWindow.listen("close_note_request", () => closeNote()),
      await appWindow.listen("tauri://move", saveGeometryDebounced),
      await appWindow.listen("tauri://resize", saveGeometryDebounced),
    );
  });

  onDestroy(() => {
    if (moveTimer !== undefined) window.clearTimeout(moveTimer);
    window.removeEventListener("keydown", createNoteWithControlN, true);
    window.removeEventListener("mouseup", finishWindowDrag, true);
    unlisteners.forEach((unlisten) => unlisten());
  });
</script>

{#if shortcutsWindow}
  <ShortcutsHelp />
{:else}
  <div class="titlebar" class:hover={titlebarHovered} class:collapsed>
  <div
    class="drag-surface"
    onmousedown={(event) => {
      if (event.button === 0 && event.detail === 1) {
        event.preventDefault();
        event.stopImmediatePropagation();
        void invoke("start_window_drag");
      }
    }}
    ondblclick={toggleCollapsed}
    onkeydown={(event) => {
      if (event.key === "Enter" || event.key === " ") void toggleCollapsed();
    }}
    role="button"
    tabindex="0"
    aria-label="Double-click to fold or unfold note"
  >
    <span class="note-title">{noteTitle}</span>
  </div>
  <div class="controls">
    <button
      class="titlebar-button"
      onclick={(event) => {
        event.stopPropagation();
        void closeNote();
      }}
      aria-label="close note"
    >
      <Icon path={mdiClose} size={15} />
    </button>
    <button
      class="titlebar-button"
      onclick={(event) => {
        event.stopPropagation();
        void toggleAlwaysOnTop();
      }}
      aria-label={alwaysOnTop ? "unpin note" : "pin note"}
    >
      <Icon path={alwaysOnTop ? mdiPinOff : mdiPin} size={10} />
    </button>
    <button
      class="titlebar-button"
      disabled={stackBusy}
      onclick={(event) => {
        event.stopPropagation();
        void linkAllNotes();
      }}
      aria-label="Link all notes to this note."
      title="Link all notes to this note."
    >
      <Icon path={mdiLinkVariant} size={11} />
    </button>
    <button
      class="titlebar-button"
      onclick={(event) => {
        event.stopPropagation();
        toggleColorMenu();
      }}
      aria-label="select note color"
    >
      <Icon path={mdiPalette} size={10} />
    </button>
    {#if colorMenuOpen}
      {#each colors as color}
        <button
          class="color"
          onclick={(event) => {
            event.stopPropagation();
            void setColor(color);
          }}
          aria-label={`set note color ${color}`}
          style:background={color}
        ></button>
      {/each}
    {/if}
  </div>
  </div>

  <main class:collapsed>
    <Editor bind:this={editor} onTitleChange={(title) => (noteTitle = title)} />
  </main>
{/if}

<style>
  .titlebar {
    align-items: center;
    display: flex;
    height: 24px;
    background: rgba(0, 0, 0, 0.07);
    box-sizing: border-box;
    position: relative;
    user-select: none;
    z-index: 2;
  }

  .titlebar.hover {
    background: rgba(0, 0, 0, 0.11);
  }

  .drag-surface {
    inset: 0;
    outline: none;
    position: absolute;
  }

  .note-title {
    font: 600 12px/24px system-ui, sans-serif;
    left: 9px;
    overflow: hidden;
    pointer-events: none;
    position: absolute;
    right: 88px;
    text-align: left;
    text-overflow: ellipsis;
    top: 0;
    white-space: nowrap;
  }

  .titlebar:not(.collapsed) .note-title {
    opacity: 0;
  }

  .controls {
    display: flex;
    flex-direction: row-reverse;
    height: 24px;
    margin-left: auto;
    opacity: 0.42;
    position: relative;
    transition: opacity 120ms ease;
    z-index: 1;
  }

  .titlebar:hover .controls,
  .titlebar.collapsed .controls {
    opacity: 0.9;
  }

  button {
    background-color: transparent;
    border: 0;
    height: 24px;
    margin: 0;
    padding: 0;
    width: 20px;
  }

  .color {
    border: 1px solid rgba(0, 0, 0, 0.24);
    border-radius: 50%;
    height: 14px;
    margin: 5px 2px;
    width: 14px;
  }

  main {
    height: calc(100vh - 24px);
  }

  main.collapsed {
    display: none;
  }
</style>
