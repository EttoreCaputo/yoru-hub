import { test } from "node:test";
import assert from "node:assert/strict";
import { transcriptionRequirement } from "../src/lib/meeting-readiness.ts";

test("local transcription waits for its model", () => {
  assert.equal(transcriptionRequirement("local", null, true), "loading");
  assert.equal(transcriptionRequirement("local", false, true), "local");
  assert.equal(transcriptionRequirement("local", true, false), null);
});

test("Gemini needs its key, not the local model", () => {
  assert.equal(transcriptionRequirement("google:gemini-3.8-flash", false, true), null);
  assert.equal(transcriptionRequirement("google:gemini-3.8-flash", true, false), "google");
});

test("unsupported and empty engines never silently become local", () => {
  for (const engine of ["", "google:", "openai:whisper-1", "other"]) {
    assert.equal(transcriptionRequirement(engine, true, true), "unsupported");
  }
});
