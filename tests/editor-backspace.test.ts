// @ts-nocheck -- Keep this Node-runner regression free of test-only packages.
import assert from "node:assert/strict";
import test from "node:test";

import { Editor, getSchema, type JSONContent } from "@tiptap/core";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { EditorState, TextSelection } from "@tiptap/pm/state";

import {
  createEditorExtensions,
  deleteEmptyParagraphAfterList,
  liftEmptyListItemOnly,
} from "../src/lib/editorExtensions.ts";

const schema = getSchema(createEditorExtensions());

function paragraph(text = ""): JSONContent {
  return {
    type: "paragraph",
    content: text ? [{ type: "text", text }] : undefined,
  };
}

function listItem(text: string): JSONContent {
  return { type: "listItem", content: [paragraph(text)] };
}

function documentWithTrailingParagraph(
  list: JSONContent,
  trailingText = "",
): ProseMirrorNode {
  return schema.nodeFromJSON({
    type: "doc",
    content: [list, paragraph(trailingText)],
  });
}

function trailingParagraphStart(document: ProseMirrorNode): number {
  const trailing = document.lastChild;
  assert.equal(trailing?.type.name, "paragraph");
  return document.content.size - trailing.nodeSize + 1;
}

test("Backspace after every list type returns to its final text block", () => {
  const cases: Array<{
    name: string;
    finalText: string;
    list: JSONContent;
  }> = [
    {
      name: "bullet list",
      finalText: "bullet",
      list: {
        type: "bulletList",
        content: [listItem("bullet")],
      },
    },
    {
      name: "numbered list",
      finalText: "numbered",
      list: {
        type: "orderedList",
        content: [listItem("numbered")],
      },
    },
    {
      name: "task ending in a nested bullet list",
      finalText: "nested bullet",
      list: {
        type: "taskList",
        content: [
          {
            type: "taskItem",
            attrs: { checked: false },
            content: [
              paragraph("task"),
              {
                type: "bulletList",
                content: [listItem("nested bullet")],
              },
            ],
          },
        ],
      },
    },
  ];

  for (const { name, finalText, list } of cases) {
    const original = documentWithTrailingParagraph(list);
    let state = EditorState.create({
      schema,
      doc: original,
      selection: TextSelection.create(
        original,
        trailingParagraphStart(original),
      ),
    });

    const handled = deleteEmptyParagraphAfterList(
      state,
      (transaction) => {
        state = state.apply(transaction);
      },
    );

    assert.equal(handled, true, name);
    assert.deepEqual(
      state.doc.toJSON(),
      schema.nodeFromJSON({ type: "doc", content: [list] }).toJSON(),
      name,
    );
    assert.equal(state.selection.$from.parent.textContent, finalText, name);
    assert.equal(state.selection.$from.parentOffset, finalText.length, name);
  }
});

test("Backspace override leaves nonempty following paragraphs to Tiptap", () => {
  const original = documentWithTrailingParagraph(
    { type: "bulletList", content: [listItem("bullet")] },
    "next",
  );
  const state = EditorState.create({
    schema,
    doc: original,
    selection: TextSelection.create(
      original,
      trailingParagraphStart(original),
    ),
  });
  let dispatched = false;

  assert.equal(
    deleteEmptyParagraphAfterList(state, () => {
      dispatched = true;
    }),
    false,
  );
  assert.equal(dispatched, false);
  assert.deepEqual(state.doc.toJSON(), original.toJSON());
});

test("Backspace removes a sole trailing nested bullet", () => {
  const editor = new Editor({
    element: null,
    immediatelyRender: false,
    extensions: createEditorExtensions(),
    content: {
      type: "doc",
      content: [
        {
          type: "taskList",
          content: [
            {
              type: "taskItem",
              attrs: { checked: false },
              content: [
                paragraph("task"),
                {
                  type: "bulletList",
                  content: [listItem("")],
                },
              ],
            },
          ],
        },
      ],
    },
  });

  try {
    let cursor: number | undefined;
    editor.state.doc.descendants((node, position) => {
      if (node.type.name === "paragraph" && node.content.size === 0) {
        cursor = position + 1;
      }
    });
    assert.notEqual(cursor, undefined);
    editor.view.dispatch(
      editor.state.tr.setSelection(
        TextSelection.create(editor.state.doc, cursor),
      ),
    );

    assert.equal(liftEmptyListItemOnly(editor), true);
    assert.deepEqual(
      editor.getJSON(),
      editor.schema.nodeFromJSON({
        type: "doc",
        content: [
          {
            type: "taskList",
            content: [
              {
                type: "taskItem",
                attrs: { checked: false },
                content: [paragraph("task")],
              },
            ],
          },
          paragraph(),
        ],
      }).toJSON(),
    );
    assert.equal(editor.state.selection.$from.depth, 1);
    assert.equal(editor.state.selection.$from.parent.content.size, 0);
  } finally {
    editor.destroy();
  }
});
