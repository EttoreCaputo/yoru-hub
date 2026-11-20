---
name: summarize-meeting
description: Write the summary of a recorded meeting, call, lesson or voice note from its transcript, in a style (auto, meeting, call, interview, lecture, note).
---
# Summarizing a meeting

1. Read the whole transcript with meeting_read, part after part until it says there is no more. Each line has the time, who spoke and what they said; the speakers are called by their names when they are known. A transcript made by speech recognition has mistakes: read past them.
2. Write the summary in the language the meeting was held in, and save it with meeting_save_summary. Say in a line that it is done; the summary itself is shown on the meeting's page, not repeated in the chat.

Be faithful: only what was said, no advice and nothing made up. Leave a list empty rather than pad it.

## The fields

- `title`: what it was about, at most eight words (never just "meeting" or "summary").
- `overview`: two to four sentences, the gist a reader needs if they read nothing else.
- `sections`: the body, laid out as the style says (below); each item one line, starting with its subject in **bold** when that helps ("**Pricing:** 39 euro a month"), with who said it when it matters.
- `decisions`: what was agreed.
- `actions`: what someone has to do, a short imperative each: `owner` as the transcript names them (or empty); `due` as said (or empty); `date` the day it is due as YYYY-MM-DD, worked out from `due` and the day it was recorded (or empty); `kind` "email" for writing an email, "message" for sending a message (chat, WhatsApp, Slack), "reminder" for something at a time (a call, an appointment), else "task"; `draft`, for an email or a message only, the text ready to send, short, in the speakers' language (else empty).
- `questions`: what was left open.

## The styles

The request names one; without one, `auto`.

- `auto`: first tell what this is (a meeting, a call, an interview, a lesson, a talk, a voice note, a chat), then lay out the sections that fit it: two to five, each named for what is in it (never "Points" or "Summary"), in the order that reads best.
- `meeting`: one section per topic discussed, named for the topic, with what was said on it and by whom.
- `call`: a short call note: a section "Background" (why the call, who was on it), then one section with what was discussed. Keep each item short.
- `interview`: one section per question asked, titled with the question, its items the key points of the answer; then a section "Impressions" if any were said.
- `lecture`: study notes: one section per topic of the lesson, in order, with its key ideas, definitions and examples; then a section "To review".
- `note`: one person speaking to themselves (a voice note, a dictation): one section per idea, in their words, without the fillers and repeats.
