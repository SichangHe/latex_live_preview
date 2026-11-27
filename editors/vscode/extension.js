// VS Code client for `ltxp`, the instant LaTeX preview language server. Nothing starts on its
// own: the command "ltxp: Open preview" starts ltxp and opens the preview in the browser.
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

const TEX = ["latex", "tex"];
let client;
/** Whether the preview follows the cursor. */
let sync = true;

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
  if (!sync || !client?.isRunning() || !editor || !TEX.includes(editor.document.languageId)) return;
  client.sendNotification("ltxp/cursor", {
    textDocument: { uri: editor.document.uri.toString() },
    position: editor.selection.active,
  });
}

async function openPreview() {
  const editor = vscode.window.activeTextEditor;
  if (!editor || !TEX.includes(editor.document.languageId)) {
    return vscode.window.showErrorMessage("ltxp: open a TeX file first");
  }
  if (!client?.isRunning()) {
    const command = findLtxp();
    if (!fs.existsSync(command)) {
      return vscode.window.showErrorMessage(`ltxp: ${command} not found; install ltxp or set ltxp.path`);
    }
    client = new LanguageClient("ltxp", "ltxp", { command, args: [] }, {
      documentSelector: TEX.map((language) => ({ scheme: "file", language })),
      initializationOptions: { openBrowser: false },
    });
    try {
      await client.start();
    } catch (e) {
      return vscode.window.showErrorMessage(`ltxp: cannot start ${command}: ${e.message ?? e}`);
    }
  }
  const url = await client.sendRequest("workspace/executeCommand", { command: "ltxp.previewUrl", arguments: [] });
  if (typeof url !== "string") {
    return vscode.window.showErrorMessage("ltxp: no preview: the file has no \\documentclass, and no main file was found");
  }
  if (!(await vscode.env.openExternal(vscode.Uri.parse(url)))) {
    vscode.window.showWarningMessage(`ltxp: cannot open a browser; open ${url} yourself`);
  }
}

function activate(context) {
  let timer;
  context.subscriptions.push(
    vscode.commands.registerCommand("ltxp.openPreview", openPreview),
    vscode.commands.registerCommand("ltxp.stop", () => client?.stop()),
    vscode.commands.registerCommand("ltxp.openLog", async () => {
      if (!client?.isRunning()) return vscode.window.showErrorMessage("ltxp: not running; run \"ltxp: Open preview\" first");
      const log = await client.sendRequest("workspace/executeCommand", { command: "ltxp.logPath", arguments: [] });
      if (typeof log !== "string") return vscode.window.showWarningMessage("ltxp: no log yet: no compile has finished");
      const doc = await vscode.workspace.openTextDocument(log);
      const ed = await vscode.window.showTextDocument(doc, vscode.ViewColumn.Beside);
      const end = new vscode.Position(doc.lineCount - 1, 0);
      ed.revealRange(new vscode.Range(end, end));
    }),
    vscode.commands.registerCommand("ltxp.toggleSync", () => {
      sync = !sync;
      vscode.window.showInformationMessage(`ltxp: preview follows cursor: ${sync ? "on" : "off"}`);
    }),
    vscode.commands.registerCommand("ltxp.forwardSearchHere", () => sendCursor(vscode.window.activeTextEditor)),
    vscode.window.onDidChangeTextEditorSelection((e) => {
      clearTimeout(timer);
      const delay = vscode.workspace.getConfiguration("ltxp").get("cursorDelayMs");
      timer = setTimeout(() => sendCursor(e.textEditor), delay);
    }),
  );
}

function deactivate() {
  return client?.stop();
}

module.exports = { activate, deactivate };
