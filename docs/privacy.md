# Yoru privacy policy

Last updated: 2026-11-08

Yoru is a personal assistant that runs on your computer, with a small round board on your desk as its body. This page says what Yoru does with your data. The short version: **Yoru works on your computer. We run no server and collect nothing about you.**

"We" is Byte Busters, the maker of Yoru.

## What stays on your computer

Everything Yoru knows lives in a folder on your computer (on macOS, `~/Library/Application Support`):

- your settings and the list of services you connected;
- the memory Yoru keeps about you (`memory.db`: the people and things in your life, the facts about them, the conversations and meeting summaries they were learnt from, how Yoru feels about each person and the voice print of each voice it knows), which you can read and edit in the app;
- your conversations, notifications and timers, and the clips Yoru spoke with ElevenLabs;
- the meetings you record or import: the sound of each track and its transcript, on the Meetings page, until you delete them. Local transcription uses models downloaded from Hugging Face; choosing Gemini sends the audio to Google for transcription. Summaries use either a local model or the selected provider, which receives the transcript. The Google guided setup selects Gemini for meeting audio and the brain for summaries; the other guided setups select local transcription and the brain for summaries.
- if you pair a Pocket recorder: its recordings, copied over Bluetooth straight to this computer and kept as meetings (nothing goes to the Pocket's maker), and the key Yoru paired it with, in the keychain.

If you turn on Yoru in the notch, he hears through the Mac's microphone only while you hold ⌥ Space or his button, or for the few seconds he listens after an answer; what he hears is treated as the board's voice is.

The sign-in tokens of your services and your API keys are kept in the macOS Keychain, never in a plain file. The API keys of the speech, brain and voice providers you picked are also copied to the board's own memory, so that it can talk with you while the computer is away (see The board, below).

We have no account system, no analytics and no telemetry. We never receive your data, your tokens or your conversations.

## What Yoru sends, and to whom

Yoru sends data only to the services you choose, and only to do what you asked:

- **The services you connect** (for example Gmail, Google Calendar, GitHub, Slack, Notion, Linear). Yoru signs in with your account, through the service's own sign-in page, and asks for the permissions listed there. It reads or acts on that service only when you ask, or when a watcher you set up checks it.
- **The model that answers you.** To think, Yoru sends your request, your memory (what is always in mind and the facts found for the request; facts marked private only to Ollama, and the people with a role for you, such as your wife, by that role rather than by name unless you name them), and the results of the tools it used (for example the mails or events it was asked about) to the model provider you picked: Groq, OpenAI, Google or Anthropic, with your own API key. On Google's free tier, Google may use what it receives to improve its products. Their handling of that data is covered by their own policies. If you pick Ollama, the model runs on your computer and nothing leaves it.
- **Gemini Live, if you turn it on.** Then what the board hears after the wake word goes to Google as audio, as you speak, with your memory's core and the results of the tools it uses; Google sends back the spoken answer and the transcription of both. On the free tier Google may use it to improve its products.
- **Your coding agents, if you ask.** A prompt you give Yoru for Claude Code, Codex or OpenCode goes to that agent's command on this computer, which sends it to its model's provider (Anthropic, OpenAI, or the one you set up in OpenCode) as it always does. To list your conversations Yoru reads their files on this computer (`~/.claude/projects`, `~/.codex/sessions`, OpenCode's database, read only); nothing of them leaves it.
- **The voice providers, if you pick them.** With an ElevenLabs or Groq voice, the text Yoru speaks is sent to that provider. Speech recognition runs on your computer, unless you pick a provider's Whisper (Groq or OpenAI): then what you say to the board is sent to it. What ElevenLabs and Gemini Live spoke is kept on your computer, on the Spoken page, until you delete it.
- **The Mac's calendar and microphone, if you turn on "Notice meetings".** Yoru reads the events of the next two hours from the calendar on this computer (after macOS asks you) and which apps are using the microphone, to ask whether to record a meeting. Nothing of it leaves the computer; nothing is recorded without your yes.
- **The services' icons.** To show a service's icon on the board (Spotify, ClickUp...), Yoru first looks for an app of that name on this computer; if there is none it downloads the public icon from the service's site (`<name>.com/apple-touch-icon.png`, Google's icon page for Gmail, Calendar, Drive and Meet). Only the service's name is in that request, nothing about you or what you asked. The icons are kept on your computer.
- **The board.** Yoru talks to the board over USB or your local WiFi. Nothing goes to the internet for that. While the computer is away, the board talks to the same speech, brain and voice providers itself, over your WiFi, with the keys the app gave it: what you say to it, a short summary of your memory (no private facts, people with a role by it) and today's timers and the names and times of your tasks go to them, as they would from the computer. The keys reach the board only over USB or its encrypted WiFi link, and stay in its flash until the app tells it to forget them (when you remove a provider).

## Acting on your behalf

Reading never asks. Anything that changes something (sending a mail, creating an event or an issue) shows a card on the board and in the app, and runs only when you press Allow.

## Google user data

Yoru's use of information received from Google APIs adheres to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including its Limited Use requirements. In particular:

- Yoru asks for access to Gmail and Google Calendar, through an OAuth client you create in your own Google Cloud project, only so that it can read them and, with your approval, act on them for you.
- Google data is used only to answer your requests and to tell you what is new, inside the app on your computer.
- It is not sold, not used for advertising, not shared with us or with anyone else, and not used to train models. The only place it goes, apart from Google, is the model provider you chose, as part of answering your request (see above).
- No person reads it: we never receive it.

## Your choices

- **Disconnect a service** in the Connectors page: Yoru forgets its tokens. You can also remove Yoru's access from your Google account at [myaccount.google.com/permissions](https://myaccount.google.com/permissions), or from the same page in the other services.
- **Edit or erase memory and conversations** in the app (a person's voice print too), or delete the data folder.
- **Delete everything**: remove the app and its data folder, and remove its entries from the keychain.

## Children

Yoru is not meant for children under 13.

## Changes

If this policy changes, the date at the top changes too, and the new version is published at the same address.

## Contact

Questions about privacy: bytebustersdev@gmail.com. The [terms of service](terms.md) are on their own page.
