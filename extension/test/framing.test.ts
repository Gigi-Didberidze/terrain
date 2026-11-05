import { test } from "node:test";
import assert from "node:assert/strict";
import { FrameDecoder, encodeFrame } from "../src/framing.ts";

test("decodes frames split across arbitrary chunk boundaries", () => {
  const a = encodeFrame(Buffer.from('{"a":"é"}'));
  const b = encodeFrame(Buffer.from('{"b":2}'));
  const all = Buffer.concat([a, b]);
  for (let split = 1; split < all.length; split++) {
    const d = new FrameDecoder();
    const got = [...d.push(all.subarray(0, split)), ...d.push(all.subarray(split))].map((x) => x.toString());
    assert.deepEqual(got, ['{"a":"é"}', '{"b":2}']);
  }
});
