// LSP-style framing: "Content-Length: N\r\n\r\n" followed by N bytes of UTF-8 JSON.

export function encodeFrame(body: Buffer): Buffer {
  return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "ascii"), body]);
}

export class FrameDecoder {
  private buf: Buffer = Buffer.alloc(0);

  /** Feeds a chunk and returns every complete message body it completes. */
  push(chunk: Buffer): Buffer[] {
    this.buf = this.buf.length === 0 ? chunk : Buffer.concat([this.buf, chunk]);
    const out: Buffer[] = [];
    for (;;) {
      const headerEnd = this.buf.indexOf("\r\n\r\n");
      if (headerEnd < 0) break;
      const header = this.buf.subarray(0, headerEnd).toString("ascii");
      const m = /content-length:\s*(\d+)/i.exec(header);
      if (!m) {
        // Unrecoverable framing error: drop the header and continue.
        this.buf = this.buf.subarray(headerEnd + 4);
        continue;
      }
      const len = Number(m[1]);
      const start = headerEnd + 4;
      if (this.buf.length < start + len) break;
      out.push(this.buf.subarray(start, start + len));
      this.buf = this.buf.subarray(start + len);
    }
    return out;
  }
}
