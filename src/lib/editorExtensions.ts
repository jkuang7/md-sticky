import { TaskItem, TaskList } from "@tiptap/extension-list";
import { Placeholder } from "@tiptap/extension-placeholder";
import { Extension, type Editor, type JSONContent } from "@tiptap/core";
import { Fragment } from "@tiptap/pm/model";
import { Selection, TextSelection } from "@tiptap/pm/state";
import { canJoin } from "@tiptap/pm/transform";
import { StarterKit } from "@tiptap/starter-kit";

function removeCheckedTasks(nodes: JSONContent[]): {
  nodes: JSONContent[];
  removed: boolean;
} {
  let removed = false;
  const remaining = nodes.flatMap((node) => {
    if (node.type === "taskItem" && node.attrs?.checked === true) {
      removed = true;
      return [];
    }

    const children = removeCheckedTasks(node.content ?? []);
    removed ||= children.removed;

    if (node.type === "taskList" && children.nodes.length === 0) {
      removed = true;
      return [];
    }

    return [
      node.content === undefined ? node : { ...node, content: children.nodes },
    ];
  });

  return { nodes: remaining, removed };
}

function activeListItemType(
  editor: Editor,
): "listItem" | "taskItem" | undefined {
  const { $from } = editor.state.selection;
  for (let depth = $from.depth; depth > 0; depth -= 1) {
    const type = $from.node(depth).type.name;
    if (type === "listItem" || type === "taskItem") return type;
  }
  return undefined;
}

function sinkListItemAcrossAdjacentLists(
  editor: Editor,
  itemType: "listItem" | "taskItem",
): boolean {
  if (editor.can().sinkListItem(itemType)) {
    return editor.commands.sinkListItem(itemType);
  }

  const { $from } = editor.state.selection;
  let itemDepth = $from.depth;
  while (itemDepth > 0 && $from.node(itemDepth).type.name !== itemType) {
    itemDepth -= 1;
  }
  if (itemDepth < 2) return false;

  const listDepth = itemDepth - 1;
  const parentDepth = listDepth - 1;
  const list = $from.node(listDepth);
  const listIndex = $from.index(parentDepth);
  if (listIndex === 0) return false;

  const previousList = $from.node(parentDepth).child(listIndex - 1);
  const joinPosition = $from.before(listDepth);
  if (previousList.type === list.type) {
    if (!canJoin(editor.state.doc, joinPosition)) return false;

    return editor
      .chain()
      .command(({ tr }) => {
        tr.join(joinPosition);
        return true;
      })
      .sinkListItem(itemType)
      .run();
  }

  const { selection } = editor.state;
  if (
    itemType !== "listItem" ||
    list.type.name !== "bulletList" ||
    previousList.type.name !== "taskList" ||
    selection.$to.depth < listDepth ||
    selection.$to.node(listDepth) !== list ||
    $from.index(listDepth) !== 0
  ) {
    return false;
  }

  const lastSelectedItem = selection.$to.index(listDepth);
  let movedSize = 0;
  for (let index = 0; index <= lastSelectedItem; index += 1) {
    movedSize += list.child(index).nodeSize;
  }

  const movedItems = list.content.cut(0, movedSize);
  const remainingItems = list.content.cut(movedSize);
  const previousTask = previousList.lastChild;
  if (!previousTask) return false;

  const existingNestedList =
    previousTask.lastChild?.type === list.type
      ? previousTask.lastChild
      : undefined;
  const nestedList = existingNestedList
    ? existingNestedList.copy(
        existingNestedList.content.append(movedItems),
      )
    : list.type.create(list.attrs, movedItems);
  const nestedContent = existingNestedList
    ? previousTask.content.replaceChild(
        previousTask.childCount - 1,
        nestedList,
      )
    : previousTask.content.append(Fragment.from(nestedList));
  if (!previousTask.type.validContent(nestedContent)) return false;

  const updatedTask = previousTask.copy(nestedContent);
  const updatedPreviousList = previousList.copy(
    previousList.content.replaceChild(
      previousList.childCount - 1,
      updatedTask,
    ),
  );
  const replacement =
    remainingItems.size > 0
      ? Fragment.fromArray([
          updatedPreviousList,
          list.copy(remainingItems),
        ])
      : Fragment.from(updatedPreviousList);

  const previousListStart = joinPosition - previousList.nodeSize;
  const previousTaskStart =
    previousListStart +
    1 +
    previousList.content.size -
    previousTask.nodeSize;
  const nestedListStart = existingNestedList
    ? previousTaskStart +
      1 +
      previousTask.content.size -
      existingNestedList.nodeSize
    : previousTaskStart + 1 + previousTask.content.size;
  const movedContentStart =
    nestedListStart +
    1 +
    (existingNestedList?.content.size ?? 0);
  const currentContentStart = joinPosition + 1;
  const selectionFrom =
    movedContentStart + selection.from - currentContentStart;
  const selectionTo =
    movedContentStart + selection.to - currentContentStart;

  return editor.commands.command(({ tr }) => {
    tr.replaceWith(
      previousListStart,
      joinPosition + list.nodeSize,
      replacement,
    );
    tr.setSelection(
      TextSelection.create(tr.doc, selectionFrom, selectionTo),
    );
    tr.scrollIntoView();
    return true;
  });
}

function liftBulletListItemOutOfTask(
  editor: Editor,
): boolean | undefined {
  const { selection } = editor.state;
  const { $from, $to } = selection;
  let itemDepth = $from.depth;
  while (
    itemDepth > 0 &&
    $from.node(itemDepth).type.name !== "listItem"
  ) {
    itemDepth -= 1;
  }
  if (itemDepth < 4) return undefined;

  const listDepth = itemDepth - 1;
  const taskDepth = listDepth - 1;
  const taskListDepth = taskDepth - 1;
  const list = $from.node(listDepth);
  const task = $from.node(taskDepth);
  const taskList = $from.node(taskListDepth);
  if (
    list.type.name !== "bulletList" ||
    task.type.name !== "taskItem" ||
    taskList.type.name !== "taskList"
  ) {
    return undefined;
  }

  if (
    $to.depth < listDepth ||
    $to.node(listDepth) !== list ||
    $from.index(taskDepth) !== task.childCount - 1 ||
    $from.index(taskListDepth) !== taskList.childCount - 1
  ) {
    return false;
  }

  const firstSelectedItem = $from.index(listDepth);
  const lastSelectedItem = $to.index(listDepth);
  if (lastSelectedItem !== list.childCount - 1) return false;

  let keptSize = 0;
  for (let index = 0; index < firstSelectedItem; index += 1) {
    keptSize += list.child(index).nodeSize;
  }

  const keptItems = list.content.cut(0, keptSize);
  const movedItems = list.content.cut(keptSize);
  const nestedContent =
    keptItems.size > 0
      ? task.content.replaceChild(
          task.childCount - 1,
          list.copy(keptItems),
        )
      : task.content.cut(0, task.content.size - list.nodeSize);
  if (!task.type.validContent(nestedContent)) return false;

  const updatedTask = task.copy(nestedContent);
  const updatedTaskList = taskList.copy(
    taskList.content.replaceChild(
      taskList.childCount - 1,
      updatedTask,
    ),
  );
  const containerDepth = taskListDepth - 1;
  const taskListIndex = $from.index(containerDepth);
  const container = $from.node(containerDepth);
  const nextList =
    taskListIndex + 1 < container.childCount &&
    container.child(taskListIndex + 1).type === list.type
      ? container.child(taskListIndex + 1)
      : undefined;
  const outdentedList = list.copy(
    nextList ? movedItems.append(nextList.content) : movedItems,
  );
  const replacement = Fragment.fromArray([
    updatedTaskList,
    outdentedList,
  ]);

  const taskListStart = $from.before(taskListDepth);
  const taskListEnd = $from.after(taskListDepth);
  const nestedListStart =
    $from.before(taskDepth) +
    1 +
    task.content.size -
    list.nodeSize;
  const oldMovedContentStart = nestedListStart + 1 + keptSize;
  const newMovedContentStart =
    taskListStart + updatedTaskList.nodeSize + 1;
  const selectionFrom =
    newMovedContentStart + selection.from - oldMovedContentStart;
  const selectionTo =
    newMovedContentStart + selection.to - oldMovedContentStart;

  return editor.commands.command(({ tr }) => {
    tr.replaceWith(
      taskListStart,
      taskListEnd + (nextList?.nodeSize ?? 0),
      replacement,
    );
    tr.setSelection(
      TextSelection.create(tr.doc, selectionFrom, selectionTo),
    );
    tr.scrollIntoView();
    return true;
  });
}

const StickyShortcuts = Extension.create({
  name: "stickyShortcuts",
  priority: 1_000,
  addKeyboardShortcuts() {
    return {
      Tab: () => {
        const itemType = activeListItemType(this.editor);
        if (itemType) {
          sinkListItemAcrossAdjacentLists(this.editor, itemType);
        }
        // Keep WebKit from moving keyboard focus into a task checkbox.
        return true;
      },
      "Shift-Tab": () => {
        const itemType = activeListItemType(this.editor);
        if (itemType === "listItem") {
          const liftedFromTask = liftBulletListItemOutOfTask(
            this.editor,
          );
          if (liftedFromTask !== undefined) return true;
          if (this.editor.commands.liftListItem("listItem")) {
            return true;
          }
        } else if (
          itemType === "taskItem" &&
          this.editor.commands.liftListItem("taskItem")
        ) {
          return true;
        }

        const { state, view } = this.editor;
        const { $from } = state.selection;
        if (
          state.selection.empty &&
          $from.depth === 1 &&
          $from.parent.isTextblock
        ) {
          const previousLine = Selection.findFrom(
            state.doc.resolve($from.before()),
            -1,
            true,
          );
          if (previousLine) {
            const currentContent = state.doc.slice(
              $from.start(),
              $from.end(),
            ).content;
            const previousEnd = previousLine.$from.end();
            const needsSpace =
              previousLine.$from.parent.textContent.length > 0 &&
              $from.parent.textContent.length > 0 &&
              !/\s$/.test(previousLine.$from.parent.textContent) &&
              !/^\s/.test($from.parent.textContent);
            const transaction = state.tr.delete(
              $from.before(),
              $from.after(),
            );
            let insertionEnd = previousEnd;
            if (needsSpace) {
              transaction.insertText(" ", insertionEnd);
              insertionEnd += 1;
            }
            transaction.insert(insertionEnd, currentContent);
            insertionEnd += currentContent.size;
            view.dispatch(
              transaction
                .setSelection(
                  TextSelection.create(transaction.doc, insertionEnd),
                )
                .scrollIntoView(),
            );
          }
        }

        // Always consume the shortcut so WebKit cannot focus titlebar controls.
        return true;
      },
      "Mod-Shift-0": () => this.editor.commands.toggleBulletList(),
      "Mod-Shift-x": () => {
        const result = removeCheckedTasks(this.editor.getJSON().content ?? []);
        if (!result.removed) return true;

        return this.editor.commands.setContent({
          type: "doc",
          content:
            result.nodes.length > 0
              ? result.nodes
              : [{ type: "paragraph" }],
        });
      },
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
