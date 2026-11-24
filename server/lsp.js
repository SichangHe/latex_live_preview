/** Minimal LSP JSON-RPC transport over stdio. */

/** Calls `onMessage(msg)` for each incoming message; returns `send(msg)`. */
export function lspTransport(onMessage) {
  let buf = Buffer.alloc(0);
  process.stdin.on("data", (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    for (;;) {
      const sep = buf.indexOf("\r\n\r\n");
      if (sep < 0) return;
      const len = Number(/Content-Length: *(\d+)/i.exec(buf.subarray(0, sep).toString())?.[1]);
      if (buf.length < sep + 4 + len) return;
      const body = buf.subarray(sep + 4, sep + 4 + len).toString();
      buf = buf.subarray(sep + 4 + len);
      onMessage(JSON.parse(body));
    }
  });
  return (msg) => {
    const body = Buffer.from(JSON.stringify({ jsonrpc: "2.0", ...msg }));
    process.stdout.write(`Content-Length: ${body.length}\r\n\r\n`);
    process.stdout.write(body);
  };
}
