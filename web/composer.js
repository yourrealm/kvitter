// Upgrades `textarea[data-composer]` to CodeMirror: markdown kept verbatim and styled in place,
// #tag autocomplete, and image upload by button, paste or drop. The textarea stays in the form
// and is kept in sync, so submitting works exactly as without JS.

import { EditorView, keymap, placeholder, Decoration, MatchDecorator, ViewPlugin } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { parser as markdownParser, GFM } from "@lezer/markdown";
import { Language, LanguageSupport, defineLanguageFacet, syntaxHighlighting, HighlightStyle } from "@codemirror/language";
import { autocompletion, completionKeymap } from "@codemirror/autocomplete";
import { tags as t } from "@lezer/highlight";

// Built from @lezer/markdown directly: @codemirror/lang-markdown would also bundle the HTML,
// CSS and JavaScript parsers for embedded code.
const markdown = new LanguageSupport(new Language(defineLanguageFacet(), markdownParser.configure(GFM), [], "markdown"));

const style = HighlightStyle.define([
  { tag: t.processingInstruction, class: "cm-md" },
  { tag: t.url, class: "cm-md-url" },
  { tag: t.link, class: "cm-md-link" },
  { tag: t.monospace, class: "cm-md-code" },
  { tag: t.strong, fontWeight: "bold" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: t.heading, fontWeight: "bold" },
  { tag: t.quote, class: "cm-md" },
]);

// Same rule as the server: # + letters/digits/_/-, with at least one letter, not glued to a word.
const TAG = /(?<![\p{L}\p{N}_&/#])#[\p{L}\p{N}_-]*\p{L}[\p{L}\p{N}_-]*/gu;
const tagMarks = new MatchDecorator({ regexp: TAG, decoration: Decoration.mark({ class: "cm-md-tag" }) });
const tagHighlighter = ViewPlugin.define(
  (view) => ({
    decorations: tagMarks.createDeco(view),
    update(u) { this.decorations = tagMarks.updateDeco(u, this.decorations); },
  }),
  { decorations: (v) => v.decorations },
);

async function tagCompletions(context) {
  const word = context.matchBefore(/#[\p{L}\p{N}_-]*/u);
  if (!word) return null;
  const before = context.state.sliceDoc(word.from - 1, word.from);
  if (before && /[\p{L}\p{N}_&/#]/u.test(before)) return null;
  try {
    const res = await fetch("/api/tags?q=" + encodeURIComponent(word.text.slice(1)));
    if (!res.ok) return null;
    const tags = await res.json();
    return {
      from: word.from,
      options: tags.map((x) => ({ label: "#" + x.tag, detail: String(x.count), type: "keyword" })),
      validFor: /^#[\p{L}\p{N}_-]*$/u,
    };
  } catch {
    return null;
  }
}

let uploadCount = 0;

function upgrade(textarea) {
  const form = textarea.form;
  const button = form.querySelector("[data-upload]");
  const input = form.querySelector("[data-upload-input]");

  function showError(message) {
    let el = form.querySelector(".error");
    if (!el) {
      el = document.createElement("p");
      el.className = "error";
      el.setAttribute("role", "alert");
      form.prepend(el);
    }
    el.textContent = message;
  }

  function replaceText(view, from, to) {
    const at = view.state.doc.toString().indexOf(from);
    if (at >= 0) view.dispatch({ changes: { from: at, to: at + from.length, insert: to } });
  }

  async function upload(view, file) {
    const marker = `![Uploading ${file.name || "image"} #${++uploadCount}…]()`;
    const { from, to } = view.state.selection.main;
    view.dispatch({ changes: { from, to, insert: marker }, selection: { anchor: from + marker.length } });
    button.setAttribute("aria-busy", "true");
    try {
      const body = new FormData();
      body.append("file", file);
      const res = await fetch("/files", { method: "POST", body });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(data.error || (res.status === 413 ? "That image is too large." : "Upload failed."));
      replaceText(view, marker, data.markdown);
    } catch (e) {
      replaceText(view, marker, "");
      showError(e.message);
    } finally {
      button.removeAttribute("aria-busy");
    }
  }

  function uploadAll(view, files) {
    const images = [...files].filter((f) => f.type.startsWith("image/"));
    images.forEach((f) => upload(view, f));
    return images.length > 0;
  }

  const view = new EditorView({
    parent: textarea.parentNode,
    state: EditorState.create({
      doc: textarea.value,
      extensions: [
        EditorView.cspNonce.of(document.querySelector("meta[name=csp-nonce]")?.content ?? ""),
        history(),
        markdown,
        syntaxHighlighting(style),
        tagHighlighter,
        autocompletion({ override: [tagCompletions], icons: false }),
        placeholder(textarea.placeholder),
        EditorView.lineWrapping,
        EditorView.contentAttributes.of({ "aria-label": "Post", spellcheck: "true", autocapitalize: "sentences" }),
        keymap.of([
          { key: "Mod-Enter", run: () => (form.requestSubmit(), true) },
          ...completionKeymap,
          ...historyKeymap,
          ...defaultKeymap,
        ]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) textarea.value = u.state.doc.toString();
        }),
        EditorView.domEventHandlers({
          paste(e, view) {
            return e.clipboardData && uploadAll(view, e.clipboardData.files) && (e.preventDefault(), true);
          },
          drop(e, view) {
            return e.dataTransfer && uploadAll(view, e.dataTransfer.files) && (e.preventDefault(), true);
          },
        }),
      ],
    }),
  });
  textarea.parentNode.insertBefore(view.dom, textarea);
  textarea.hidden = true;

  button.hidden = false;
  button.addEventListener("click", () => input.click());
  input.addEventListener("change", () => {
    uploadAll(view, input.files);
    input.value = "";
    view.focus();
  });

  if (!form.querySelector(".error") && textarea.value === "") view.focus();
  else view.dispatch({ selection: { anchor: view.state.doc.length } });
}

document.querySelectorAll("textarea[data-composer]").forEach(upgrade);
