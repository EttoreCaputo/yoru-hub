import { test } from "node:test";
import assert from "node:assert/strict";
import { profileDefaults, PROFILES } from "../src/lib/profile-defaults.ts";

test("each provider gets a supported default and only Google handles meeting audio", () => {
  assert.deepEqual(PROFILES, ["google", "openai", "anthropic", "groq"]);
  const google = profileDefaults("google", "small");
  assert.equal(google.live, "Puck");
  assert.equal(google.meetingTranscriber, "google:gemini-3.8-flash");
  for (const provider of ["openai", "anthropic", "groq"]) {
    const p = profileDefaults(provider, "small");
    assert.equal(p.live, "");
    assert.equal(p.meetingTranscriber, "local");
    assert.ok(p.brainModel);
  }
  assert.equal(profileDefaults("openai", "small").hearing, "openai:whisper-1");
  assert.equal(profileDefaults("groq", "small").hearing, "groq:whisper-large-v3-turbo");
  assert.equal(profileDefaults("anthropic", "medium").hearing, "medium");
});
