// @ts-nocheck -- Keep this Node-runner regression free of test-only packages.
import assert from "node:assert/strict";
import test from "node:test";

import { getSchema, type JSONContent } from "@tiptap/core";
import { history, undo } from "@tiptap/pm/history";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { EditorState, TextSelection } from "@tiptap/pm/state";

import {
  createEditorExtensions,
  transformSelectedListItems,
} from "../src/lib/editorExtensions.ts";

const schema = getSchema(createEditorExtensions());

function paragraph(text = "", marks?: JSONContent["marks"]): JSONContent {
  return {
    type: "paragraph",
    content: text ? [{ type: "text", text, marks }] : undefined,
  };
}

function item(
  type: "listItem" | "taskItem",
  text: string,
  content: JSONContent[] = [],
  checked = false,
): JSONContent {
  return {
    type,
    attrs: type === "taskItem" ? { checked } : undefined,
    content: [paragraph(text), ...content],
  };
}

function list(
  type: "bulletList" | "taskList",
  items: JSONContent[],
): JSONContent {
  return { type, content: items };
}

function doc(...content: JSONContent[]): ProseMirrorNode {
  return schema.nodeFromJSON({ type: "doc", content });
}

function textPosition(document: ProseMirrorNode, text: string): number {
  let result: number | undefined;
  document.descendants((node, position) => {
    if (result === undefined && node.isText && node.text === text) {
      result = position;
    }
  });
  assert.notEqual(result, undefined, `Could not find text ${JSON.stringify(text)}`);
  return result;
}

function emptyParagraphPosition(document: ProseMirrorNode): number {
  let result: number | undefined;
  document.descendants((node, position) => {
    if (node.type.name === "paragraph" && node.content.size === 0) {
      result = position + 1;
    }
  });
  assert.notEqual(result, undefined, "Could not find an empty paragraph");
  return result;
}

function applyConversion(
  document: ProseMirrorNode,
  from: number,
  to: number,
  target: "bulletList" | "taskList",
): EditorState {
  let state = EditorState.create({
    schema,
    doc: document,
    selection: TextSelection.create(document, from, to),
    plugins: [history()],
  });
  const handled = transformSelectedListItems(
    state,
    (transaction) => {
      state = state.apply(transaction);
    },
    target,
  );
  assert.equal(handled, true);
  return state;
}

function listTypes(document: ProseMirrorNode): string[] {
  return document.content.content.map((node) => node.type.name);
}

test("converting the final nested empty bullet preserves its depth and siblings", () => {
  const original = doc(
    list("bulletList", [
      item("listItem", "first"),
      item("listItem", "parent", [
        list("bulletList", [
          item("listItem", "nested before"),
          item("listItem", ""),
        ]),
      ]),
    ]),
  );
  const cursor = emptyParagraphPosition(original);
  const originalDepth = original.resolve(cursor).depth;
  const state = applyConversion(original, cursor, cursor, "taskList");

  assert.equal(state.selection.$from.depth, originalDepth);
  assert.equal(state.selection.$from.parentOffset, 0);
  assert.deepEqual(
    state.doc.toJSON(),
    doc(
      list("bulletList", [
        item("listItem", "first"),
        item("listItem", "parent", [
          list("bulletList", [item("listItem", "nested before")]),
          list("taskList", [item("taskItem", "")]),
        ]),
      ]),
    ).toJSON(),
  );
});

test("first, middle, and last items convert independently in both directions", () => {
  for (const [sourceList, sourceItem, targetList, targetItem] of [
    ["bulletList", "listItem", "taskList", "taskItem"],
    ["taskList", "taskItem", "bulletList", "listItem"],
  ] as const) {
    for (const [index, expectedTypes] of [
      [0, [targetList, sourceList]],
      [1, [sourceList, targetList, sourceList]],
      [2, [sourceList, targetList]],
    ] as const) {
      const original = doc(
        list(sourceList, [
          item(sourceItem, "a", [], true),
          item(sourceItem, "b", [], true),
          item(sourceItem, "c", [], true),
        ]),
      );
      const label = ["a", "b", "c"][index];
      const position = textPosition(original, label);
      const state = applyConversion(
        original,
        position,
        position,
        targetList,
      );

      assert.deepEqual(listTypes(state.doc), expectedTypes);
      const converted = state.doc.child(index === 0 ? 0 : 1).firstChild;
      assert.equal(converted?.type.name, targetItem);
      if (targetItem === "taskItem") {
        assert.equal(converted?.attrs.checked, false);
      }
    }
  }
});

test("a range converts every touched item", () => {
  const original = doc(
    list("bulletList", [
      item("listItem", "a"),
      item("listItem", "b"),
      item("listItem", "c"),
      item("listItem", "d"),
    ]),
  );
  const from = textPosition(original, "b");
  const to = textPosition(original, "c") + 1;
  const state = applyConversion(original, from, to, "taskList");

  assert.deepEqual(listTypes(state.doc), [
    "bulletList",
    "taskList",
    "bulletList",
  ]);
  assert.equal(state.doc.child(0).textContent, "a");
  assert.equal(state.doc.child(1).textContent, "bc");
  assert.equal(state.doc.child(2).textContent, "d");
  assert.equal(state.selection.from, textPosition(state.doc, "b"));
  assert.equal(state.selection.to, textPosition(state.doc, "c") + 1);
});

test("a converted item preserves formatting and nested child content", () => {
  const nested = list("bulletList", [item("listItem", "child")]);
  const original = doc(
    list("bulletList", [
      item("listItem", "a"),
      {
        type: "listItem",
        content: [
          paragraph("bold b", [{ type: "bold" }]),
          nested,
        ],
      },
      item("listItem", "c"),
      item("listItem", "d"),
    ]),
  );
  const cursor = textPosition(original, "bold b") + 2;
  const state = applyConversion(original, cursor, cursor, "taskList");

  assert.deepEqual(listTypes(state.doc), [
    "bulletList",
    "taskList",
    "bulletList",
  ]);
  assert.deepEqual(
    state.doc.child(1).toJSON(),
    doc(
      list("taskList", [
        {
          type: "taskItem",
          attrs: { checked: false },
          content: [
            paragraph("bold b", [{ type: "bold" }]),
            nested,
          ],
        },
      ]),
    ).firstChild?.toJSON(),
  );
  assert.equal(state.selection.from, textPosition(state.doc, "bold b") + 2);
  assert.equal(state.selection.to, state.selection.from);
});

test("same-type shortcuts preserve nested items and their depth", () => {
  for (const [listType, itemType] of [
    ["bulletList", "listItem"],
    ["taskList", "taskItem"],
  ] as const) {
    const original = doc(
      list(listType, [
        item(
          itemType,
          "parent",
          [list(listType, [item(itemType, "child", [], true)])],
          true,
        ),
      ]),
    );
    const cursor = textPosition(original, "child") + 2;
    const originalDepth = original.resolve(cursor).depth;
    const state = applyConversion(original, cursor, cursor, listType);

    assert.deepEqual(state.doc.toJSON(), original.toJSON());
    assert.equal(state.selection.from, cursor);
    assert.equal(state.selection.$from.depth, originalDepth);
  }
});

test("one undo restores the complete document after a conversion", () => {
  const original = doc(
    list("bulletList", [
      item("listItem", "a"),
      item("listItem", "b"),
      item("listItem", "c"),
    ]),
  );
  const cursor = textPosition(original, "b");
  let state = applyConversion(original, cursor, cursor, "taskList");

  assert.equal(
    undo(state, (transaction) => {
      state = state.apply(transaction);
    }),
    true,
  );
  assert.deepEqual(state.doc.toJSON(), original.toJSON());
  assert.equal(undo(state), false);
});
