// Runs suite.cjs inside a real VS Code with the ltxp extension (editors/vscode) and the
// release ltxp binary. Usage: xvfb-run -a node test/vscode/run.cjs
const path = require("node:path");
const fs = require("node:fs");
const os = require("node:os");
const { runTests } = require("@vscode/test-electron");

const ROOT = path.resolve(__dirname, "../..");
const work = fs.mkdtempSync(path.join(os.tmpdir(), "ltxp-vscode-"));
fs.cpSync(path.join(ROOT, "test/multifile"), work, { recursive: true });
runTests({
  version: process.env.VSCODE_VERSION ?? "stable",
  extensionDevelopmentPath: path.join(ROOT, "editors/vscode"),
  extensionTestsPath: path.join(__dirname, "suite.cjs"),
  launchArgs: [work, "--disable-extensions", "--disable-workspace-trust", "--user-data-dir", path.join(work, ".ud")],
  extensionTestsEnv: { LTXP_PORT: "47311", LTXP_TEST_DIR: work, PATH: `${path.join(ROOT, "target/release")}:${process.env.PATH}` },
})
  .then(() => fs.rmSync(work, { recursive: true, force: true }))
  .catch((e) => {
    console.log(`FAIL vscode: ${e}; kept ${work}`);
    process.exit(1);
  });
