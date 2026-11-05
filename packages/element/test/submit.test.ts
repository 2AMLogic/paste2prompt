import assert from "node:assert/strict";
import { test } from "node:test";
import { buildSubmitPayload, runSubmit, submitEnabled } from "../src/submit.ts";

const source = () => ({
  caption: () => "1. hint",
  documentJson: () => '{"regions":[1]}',
  renderPng: async () => new Blob(["png"], { type: "image/png" }),
});

test("no button without both a notice and a callback", () => {
  const cb = () => {};
  assert.equal(submitEnabled(undefined, cb), false);
  assert.equal(submitEnabled(null, cb), false);
  assert.equal(submitEnabled("   ", cb), false);
  assert.equal(submitEnabled("Sends to the dashboard", undefined), false);
  assert.equal(submitEnabled("Sends to the dashboard", "not a function"), false);
  assert.equal(submitEnabled("Sends to the dashboard", cb), true);
});

test("the payload has exactly png, caption, document and page", async () => {
  const payload = await buildSubmitPayload(source(), "https://x.test/a#/r");
  assert.deepEqual(Object.keys(payload).sort(), ["caption", "document", "page", "png"]);
  assert.ok(payload.png instanceof Blob);
  assert.equal(payload.caption, "1. hint");
  assert.deepEqual(payload.document, { regions: [1] });
  assert.equal(payload.page, "https://x.test/a#/r");
});

test("runSubmit calls the handler once per click, and only then", async () => {
  let calls = 0;
  // Building the handler's inputs alone must not call it.
  const handler = () => {
    calls++;
  };
  await buildSubmitPayload(source(), "p");
  assert.equal(calls, 0);
  assert.equal(await runSubmit(handler, source(), "p"), undefined);
  assert.equal(calls, 1);
});

test("a throwing or rejecting handler yields an error message", async () => {
  const quiet = console.error;
  console.error = () => {};
  try {
    const thrown = await runSubmit(() => {
      throw new Error("boom");
    }, source(), "p");
    assert.equal(thrown, "Submit failed: boom");
    const rejected = await runSubmit(() => Promise.reject(new Error("nope")), source(), "p");
    assert.equal(rejected, "Submit failed: nope");
  } finally {
    console.error = quiet;
  }
});
