// VS Code client for `ltxp`, the instant LaTeX preview language server.
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

let client;

/** `ltxp.path`, else `ltxp` on PATH, else `~/.local/bin/ltxp`. */
function findLtxp() {
  const configured = vscode.workspace.getConfiguration("ltxp").get("path");
  if (configured) return configured;
  for (const dir of (process.env.PATH ?? "").split(path.delimiter)) {
    const p = path.join(dir, "ltxp");
    if (fs.existsSync(p)) return p;
  }
  return path.join(os.homedir(), ".local", "bin", "ltxp");
}

function sendCursor(editor) {
  if (!client || !editor || !["latex", "tex"].includes(editor.document.languageId)) return;
  client.sendNotification("ltxp/cursor", {
    textDocument: { uri: editor.document.uri.toString() },
    position: editor.selection.active,
  });
}

async function activate(context) {
  const command = findLtxp();
  client = new LanguageClient(
    "ltxp",
    "ltxp",
    { command, args: [] },
    { documentSelector: [{ scheme: "file", language: "latex" }, { scheme: "file", language: "tex" }] },
  );
  let timer;
  context.subscriptions.push(
    vscode.window.onDidChangeTextEditorSelection((e) => {
      clearTimeout(timer);
      const delay = vscode.workspace.getConfiguration("ltxp").get("cursorDelayMs");
      timer = setTimeout(() => sendCursor(e.textEditor), delay);
    }),
    vscode.commands.registerCommand("ltxp.forwardSearchHere", () => sendCursor(vscode.window.activeTextEditor)),
  );
  try {
    await client.start();
  } catch (e) {
    vscode.window.showErrorMessage(`ltxp: cannot start ${command}: ${e.message ?? e}. Install it or set ltxp.path.`);
  }
}

function deactivate() {
  return client?.stop();
}

module.exports = { activate, deactivate };
