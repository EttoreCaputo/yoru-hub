# Yoru privacy policy

Last updated: 2026-10-29

Yoru is a personal assistant that runs on your computer, with a small round board on your desk as its body. This page says what Yoru does with your data. The short version: **Yoru works on your computer. We run no server and collect nothing about you.**

"We" is Byte Busters, the maker of Yoru.

## What stays on your computer

Everything Yoru knows lives in a folder on your computer (on macOS, `~/Library/Application Support`):

- your settings and the list of services you connected;
- the memory Yoru keeps about you (`memory.md`), which you can read and edit in the app;
- your conversations, notifications and timers, and the clips Yoru spoke with ElevenLabs;
- the meetings you record or import: the sound of each track and its transcript, on the Meetings page, until you delete them. Recording and transcribing them never sends anything anywhere (the models are downloaded once, from Hugging Face). Their summaries are written on this computer too, unless you pick the brain for them in the meetings' settings: then the transcript goes to that brain's provider.

The sign-in tokens of your services and your API keys are kept in the macOS Keychain, never in a plain file.

We have no account system, no analytics and no telemetry. We never receive your data, your tokens or your conversations.

## What Yoru sends, and to whom

Yoru sends data only to the services you choose, and only to do what you asked:

- **The services you connect** (for example Gmail, Google Calendar, GitHub, Slack, Notion, Linear). Yoru signs in with your account, through the service's own sign-in page, and asks for the permissions listed there. It reads or acts on that service only when you ask, or when a watcher you set up checks it.
- **The model that answers you.** To think, Yoru sends your request, your memory, and the results of the tools it used (for example the mails or events it was asked about) to the model provider you picked: Groq, OpenAI or Anthropic, with your own API key. Their handling of that data is covered by their own policies. If you pick Ollama, the model runs on your computer and nothing leaves it.
- **The voice providers, if you pick them.** With an ElevenLabs or Groq voice, the text Yoru speaks is sent to that provider. Speech recognition runs on your computer, unless you pick a provider's Whisper (Groq or OpenAI): then what you say to the board is sent to it. What ElevenLabs spoke is kept on your computer, on the Spoken page, until you delete it.
- **The board.** Yoru talks to the board over USB or your local WiFi. Nothing goes to the internet for that.

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
- **Edit or erase memory and conversations** in the app, or delete the data folder.
- **Delete everything**: remove the app and its data folder, and remove its entries from the keychain.

## Children

Yoru is not meant for children under 13.

## Changes

If this policy changes, the date at the top changes too, and the new version is published at the same address.

## Contact

Questions about privacy: bytebustersdev@gmail.com. The [terms of service](terms.md) are on their own page.
