import { TaskItem, TaskList } from "@tiptap/extension-list";
import { Placeholder } from "@tiptap/extension-placeholder";
import { Extension } from "@tiptap/core";
import { Selection } from "@tiptap/pm/state";
import { StarterKit } from "@tiptap/starter-kit";

const StickyShortcuts = Extension.create({
  name: "stickyShortcuts",
  priority: 1_000,
  addKeyboardShortcuts() {
    return {
      Tab: () => {
        if (this.editor.isActive("taskItem")) {
          this.editor.commands.sinkListItem("taskItem");
          return true;
        }
        if (this.editor.isActive("listItem")) {
          this.editor.commands.sinkListItem("listItem");
          return true;
        }
        // Keep WebKit from moving keyboard focus into a task checkbox.
        return true;
      },
      "Shift-Tab": () => {
        if (
          this.editor.isActive("taskItem") &&
          this.editor.commands.liftListItem("taskItem")
        ) {
          return true;
        }
        if (
          this.editor.isActive("listItem") &&
          this.editor.commands.liftListItem("listItem")
        ) {
          return true;
        }

        const { state, view } = this.editor;
        const { $from } = state.selection;
        if ($from.depth > 0) {
          const previousLine = Selection.findFrom(
            state.doc.resolve($from.before()),
            -1,
            true,
          );
          if (previousLine) {
            view.dispatch(state.tr.setSelection(previousLine).scrollIntoView());
          }
        }

        // Always consume the shortcut so WebKit cannot focus titlebar controls.
        return true;
      },
      "Mod-Shift-0": () => this.editor.commands.toggleBulletList(),
      "Mod-Shift-c": () => {
        if (!this.editor.isActive("taskItem")) return false;
        const checked = Boolean(
          this.editor.getAttributes("taskItem").checked,
        );
        return this.editor.commands.updateAttributes("taskItem", {
          checked: !checked,
        });
      },
      "Mod-Shift-s": () => {
        const { selection } = this.editor.state;
        if (!selection.empty) return this.editor.commands.toggleStrike();

        const cursor = selection.from;
        const blockStart = selection.$from.start();
        const blockEnd = selection.$from.end();
        if (blockStart === blockEnd) {
          return this.editor.commands.toggleStrike();
        }

        const strike = this.editor.schema.marks.strike;
        let hasStrike = false;
        this.editor.state.doc.nodesBetween(blockStart, blockEnd, (node) => {
          if (node.isText && strike && strike.isInSet(node.marks)) {
            hasStrike = true;
          }
        });

        const chain = this.editor
          .chain()
          .setTextSelection({ from: blockStart, to: blockEnd });
        if (hasStrike) chain.unsetStrike();
        else chain.setStrike();
        return chain.setTextSelection(cursor).run();
      },
    };
  },
});

export function createEditorExtensions() {
  return [
    StarterKit.configure({
      heading: {
        levels: [1, 2, 3],
      },
    }),
    TaskList,
    TaskItem.configure({
      nested: true,
    }),
    StickyShortcuts,
    Placeholder.configure({
      placeholder: "Empty Note",
    }),
  ];
}
