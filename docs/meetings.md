# Meetings: record, transcribe, summarize

Decided on 2026-10-29. Yoru records what you hear and say on the computer (a call, a video, a talk), transcribes it, tells who said what, and summarizes it. Everything runs on this computer by default; a provider is an option, never a requirement. The model is MacWhisper (Good Snooze, "Whisper Transcription" on the Mac App Store), studied from the installed app 15.8; Yoru adds what only an assistant with a body can do: it notices the call, asks on the board, turns action items into tasks, and answers questions about past meetings.

macOS only, like the rest of Yoru (see "Left" in [assistant.md](assistant.md)): the capture uses Core Audio's process taps and the models run on Core ML.

## What MacWhisper does, and how

From the app's bundle (frameworks, bundled models, entitlements, interface strings):
- **Capture**: RecordKit, on Core Audio process taps (macOS 14.4+, the "System Audio Recording Only" permission, `NSAudioCaptureUsageDescription`) for all system audio or one app, and AVCapture for microphones. Each source is a track of its own; echo cancellation on the microphone with Apple's VoiceProcessingIO.
- **Transcription**: whisper.cpp (Metal) with Large v3 Turbo (626 MB), Large v3, Distil and the Silero v6.2 VAD; WhisperKit (Core ML); NVIDIA Parakeet v2 (English) and **v3 (multilingual, Italian included, 494 MB)**, its recommended engine and the only one for live transcription; Qwen3-ASR (experimental, unstable on M1); Apple's SpeechTranscriber.
- **Speakers**: Argmax SpeakerKit, pyannote v4 (community-1) for finished recordings and **Nemotron 3 Diarization** (NVIDIA Sortformer, up to 8 speakers, streaming) for live meetings. A library of named speakers: rename, merge, recolor, detect again, default participants, meeting titles from the calendar.
- **Summaries**: title and summary on completion, action items, custom prompts, prompts per app; local through Ollama or LM Studio, Apple Intelligence only for short texts.
- **The rest**: meeting detection (Zoom, Teams, Meet…) with a notification, watched folders, imported files, exports, Notion, live captions.

Argmax's Core ML builds (ParakeetKit Pro, SpeakerKit Pro) are under a commercial license. The same open models exist as Core ML builds for [FluidAudio](https://github.com/FluidInference/FluidAudio) (Swift, Apache-2.0), which is what Yoru uses.

## The models (all local by default)

| Job | Default | Alternative |
|---|---|---|
| Transcription after the meeting | Parakeet Ultra (FluidAudio, Neural Engine; CC-BY-4.0): the post-training of Parakeet TDT v3 0.6B that FluidAudio recommends over v3, more accurate in every language it measures, as fast | Whisper Large v3 Turbo on the `whisper-server` Yoru already ships (better with names and a vocabulary prompt) |
| Live transcription | Parakeet v3 streaming (FluidAudio) | [Nemotron 3.5 ASR Streaming Multilingual](https://huggingface.co/FluidInference/Nemotron-3.5-ASR-Streaming-Multilingual-0.6b-CoreML) (Italian 5.4% WER on FLEURS), once its gated access is granted |
| Voice activity | Silero VAD | |
| Speakers, live | [Nemotron 3 Diarization](https://huggingface.co/FluidInference/nemotron-3-diarization-coreml), preset `fast32` (2.9 s latency; OpenMDW-1.1) | |
| Speakers, after the meeting | Nemotron 3 Diarization, preset `fast128` (best accuracy and speaker counting) | pyannote community-1 (FluidAudio) above 8 speakers |
| Who is who across meetings | Voice prints from the pyannote embedder (WeSpeaker), matched by cosine similarity | |
| Echo on the microphone | VoiceProcessingIO | |
| Summary | The local model on `llama-server`, with a context long enough for an hour of talk (about 15,000 tokens; the assistant's is 8,192), or Ollama | The brain picked in Settings (Claude, OpenAI, Groq) when the user turns it on |

Nemotron 3 Diarization says who speaks when, but gives no voice print to keep: the pyannote embedder runs on each speaker's segments to name them.

The speed figures on the model cards are from an M5 Pro; the first test is on the user's M1 Pro, where MacWhisper itself warns that some Neural Engine models stop on long files.

Leftovers of the retired Yoru app are in `~/Library/Application Support/Yoru/models/` (FluidAudio's pyannote diarization and Parakeet v3). Yoru Hub downloads its own, pinned like the other models, into its data folder.

## How it fits Yoru Hub

```
 sources                    yoru-audio (Swift)                 yoru-core (Rust)              app, board
 system audio ─┐  tap   ┌─ tracks (mic, others) ──► files ─► meetings/ (store) ─────► Meetings page
 one app ──────┼───────►│─ live: VAD, Parakeet,     events ─► meetings.rs ──► llm ──► summary, tasks
 microphones ──┘  AVAE  └─ Nemotron (streaming)  ─(stdout)─►   agent tools ──────────► board, inbox
 audio/video file ─────────► after: Parakeet or Whisper, Nemotron, voice prints
```

- **`yoru-audio`**, a Swift helper next to the app (`audio/`, built by `tools/build_sidecars.sh`, in `externalBin`). It captures, transcribes and diarizes; Swift reaches Core Audio's taps, AVAudioEngine and FluidAudio without bindings. Yoru Hub starts it as a child process, so macOS asks the permissions for Yoru. Commands come as JSON lines on stdin, events (levels, segments, speakers, progress, errors) as JSON lines on stdout, like the other sidecars' simple contract.
- **`core/src/meetings.rs`** (and `meetings/`): the store, the summary, the agent's tools, the board's side.
- **Permissions**: `NSAudioCaptureUsageDescription` and `NSMicrophoneUsageDescription` in the app's `Info.plist`. The signed build (`tools/build_app.sh`) keeps the grant from one build to the next.

## Recording

- **Sources**: all system audio, one app (picked from the running ones), one or more microphones, together. Each source is a track; the microphone is "you", the rest "the others", so two speakers are told apart for free and diarization only runs on the others' track.
- **Start and stop**: in the app, from the board (a menu entry, or "Yoru, record the meeting"), and from the menu bar. The board shows a recording mark while it lasts, and Yoru can say that it is recording.
- **Files**: written as they come (a crash keeps what was recorded), compressed to AAC when done.
- **Meeting detection**: when an app starts using the microphone (Zoom, Meet in a browser, Teams, FaceTime…), Yoru asks on the board whether to record. Off by default.
- **Imported files**: audio or video dropped on the Meetings page or opened with the app, and an optional watched folder.

## Transcription

- **Live or after**: a setting, and a choice when a recording starts. Live shows the text and speakers while it runs; after transcribes the whole file once it stops, with the best preset. Live can still run the full pass at the end to replace the live text (on by default).
- **Language**: Italian or English, detected, or fixed in Settings.
- **The page**: segments with speaker, time and text; click to play from there; edit the text; search; export as Markdown, SRT or plain text.

## Speakers

- Each speaker gets a name and a color. Name "Speaker 2" once and Yoru keeps their voice print; next time they are named on their own (above a similarity threshold, else suggested).
- Rename, merge two speakers, move a segment to another speaker, fix the number of speakers and run it again.
- The calendar's event at the time of the recording (Google Calendar connector) gives the title and the participants as suggested names.

## Summary

- On completion (setting): a title, a short summary, decisions, action items with owners, open questions. Custom prompts, one picked per meeting.
- A long meeting that doesn't fit the model's context is summarized in parts, then the parts together.
- **Action items become work**: Yoru offers to create them as ClickUp tasks or calendar events through the connectors, on the Allow card like any acting tool.
- A notification in the inbox and on the board when the summary is ready.

## The assistant

- New tools: `list_meetings`, `search_meetings` (text and speakers), `get_meeting` (summary, or the transcript around a match), `start_recording`, `stop_recording`. "What did Marco say about the deadline?" is answered from the transcripts.
- What a meeting decided can go to memory, if the user asks.

## Privacy

Recordings, transcripts, voice prints and summaries stay in the data folder (`meetings/`, `voices.json`). A cloud transcription or summary sends the audio or the text to that provider only when the user picked it. Recording people may need their consent: the app says so the first time, and Yoru can announce the recording. `privacy.md` and `licenses.md` are updated with the first phase that ships each part.

## Phases

Status (2026-10-30): M1 is built and recorded a real call, except the board's mark and the menu bar. M2 is built with Parakeet; Whisper as an alternative is left. M3 tells the speakers apart and lets the user name them; voice prints across meetings and names from the calendar are left. M4 writes the summary; turning action items into tasks and the agent's tools are left.

- **M1. Recording.** `yoru-audio` with taps and microphones, the permissions, the store, the Meetings page with start, stop and playback, the board's mark. Done when a call is recorded as two tracks and plays back in the app.
  - Built: `audio/` (`yoru-audio sources | record | finalize`: a Core Audio tap through a private aggregate device for the computer's sound, an IOProc per microphone, AAC in CAF while recording and M4A after, levels four times a second), `core/src/meetings.rs` (the store in `meetings/<id>/meeting.json`, the recording, recovery of one cut short), `app/src-tauri/src/meetings.rs` and the Meetings page (the sources as app icons). A track that gets no sound for three seconds says so (`no_system_audio`: macOS gives no sound and no error without the System Audio Recording permission; the page opens that pane), and is kept without a file.
  - Left: the board's mark while recording (a firmware change), start and stop from the board and the menu bar, echo cancellation. One app's sound is taken from the processes it has when the recording starts: a browser tab that starts playing later isn't in it.
- **M2. Transcription after.** Parakeet v3 and Whisper Large v3 Turbo with VAD, the transcript page, exports, imported files. Done when an hour of Italian call is transcribed on the M1 Pro and shown with timestamps.
  - Built (2026-10-29): `yoru-audio model` (Parakeet Ultra into `models/parakeet-ultra`, about 630 MB, from Hugging Face), `yoru-audio transcribe` (FluidAudio 0.17.7; words grouped into segments at pauses and sentence ends), `yoru-audio import` (an audio or video file's sound as `others.m4a`), `core/src/meetings/transcript.rs` (the transcript per track, merged in time order, in `transcript.json`; Markdown, SRT and text to Downloads; `meetings/settings.json`), and on the Meetings page the transcript (a line's time plays the track from there), the exports, Import a file and dropping files on the window, and a Settings panel: when to transcribe (after the meeting, by default; when asked; live is shown but waits for M5), the language, the model's download. A recording or an import is transcribed on its own once it ends, when the model is there. 34 s of Italian took 0.6 s on the M1 Pro.
  - Left: Whisper Large v3 Turbo as the alternative engine, and a watched folder.
- **M3. Speakers.** Nemotron 3 Diarization on the others' track, voice prints, the speakers' library, names from the calendar. Done when a recurring participant is named on their own.
  - Built (2026-10-30): `yoru-audio transcribe --speakers <folder>` runs Nemotron 3 Diarization (preset `fast128`, FluidAudio, about 190 MB in `models/nemotron-3-diarization`, fetched on first use) on every track and gives each word the speaker most active over it; segments break where the speaker changes. The transcript's lines carry `speaker` (within their track); `Line::who` (`<track>/<speaker>`) keys the names the user gives in `meeting.json` (`speakers`). The page numbers the people as they first speak (the microphone's first voice is "you"), gives each a color, names them with a click, and exports use the names. A setting turns it off; "Transcribe again" redoes an older transcript. A 66 s two-person video took 1.6 s with the models loaded.
  - Left: voice prints across meetings (the pyannote embedder), names from the calendar, pyannote above 8 people.
- **M4. Summary and actions.** The local summary in parts, the prompts, action items to ClickUp and the calendar with Allow, the agent's tools.
  - Built (2026-10-30): `core/src/meetings/summary.rs` asks a language model for a title, an overview, the points, the decisions, the things to do (who, by when) and the open questions, as JSON, in `summary.json`; a meeting without a title takes the summary's. The transcript goes with the speakers' names; one longer than the model reads at once (12,000 characters for the local model and for Groq and Ollama, 120,000 for Claude and OpenAI) is summarized in parts, then the parts' notes together. The language is told to the model by name (`language_of`, from the transcript's common words): Qwen3.5 4B wrote in English otherwise. The model is a setting: this computer (default; Qwen3.5 4B on `llama-server`, the voice's own when it runs one locally, since two can't share the pid file; the page offers the 2.7 GB download) or the assistant's brain. On its own after the transcript (setting), or Summarize; Copy puts it on the clipboard as Markdown. The Boldrin video's 66 s took about 11 s on the M1 Pro.
  - Left: action items to ClickUp tasks and calendar events with Allow, the agent's tools (`search_meetings`…), the notification when it's ready.
- **M5. Live and detection.** Streaming transcription and diarization, live text in the app, the full pass at the end, meeting detection with the question on the board.

## Open questions

- Whether live text belongs on the board too (a line under the eyes) or only in the app.
- Nemotron 3.5 ASR Streaming as the live default, once its access is granted and it is measured against Parakeet v3 in Italian.
- Echo cancellation when the user has no headphones: VoiceProcessingIO on the microphone, or removing the others' track from the microphone's.
