# StickyMD

A local Markdown sticky-note app for Apple Silicon Macs. The installed app is named **Sticky**.

https://github.com/user-attachments/assets/7b61d4fa-8e2a-4b80-af09-37120dd7e8cb

## Install

Open **Terminal**, paste this entire command, and press **Return**:

```sh
/bin/bash -c "$(/usr/bin/curl -fsSL https://raw.githubusercontent.com/jkuang7/StickyMD/main/scripts/bootstrap-macos.sh)"
```

That command handles everything and opens Sticky when it is done. The first installation can take several minutes. If your Mac asks to install developer tools, click **Install**, wait for it to finish, then return to Terminal and press **Return**.

## Update

In Sticky, choose **Help → Version…**. Sticky checks the current GitHub `main` commit only when that window opens. If a different build is available, click **Update**; the installation runs visibly in Terminal, then Sticky reopens when it is finished. Updating Sticky does not replace your notes.

## If macOS blocks Sticky

Sticky is built on your Mac and is not notarized by Apple. Try to open Sticky once, then go to **System Settings → Privacy & Security**, scroll to **Security**, and click **Open Anyway**.

## Your notes

Sticky has no account, analytics, or cloud sync. Notes stay on your Mac in:

```text
~/Library/Application Support/local.jian.mdsticky/
```

Press `Command-/` inside Sticky to see its keyboard shortcuts.

## Uninstall

Quit Sticky, move `/Applications/Sticky.app` to the Trash, and delete `~/StickyMD`. Your saved notes remain in the folder above unless you delete it too.

Development and architecture details are in [PLOT.md](PLOT.md).
