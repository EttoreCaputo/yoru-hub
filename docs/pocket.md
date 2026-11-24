# The Pocket recorder

The Pocket (Open Vision Engineering's PKT01, heypocket.com) is a clip-on voice
recorder. Its own app syncs it over Bluetooth to a phone and sends the
recordings to the vendor's cloud. Yoru Hub takes that app's place: it pairs
with the Pocket with a key of its own, copies the recordings to this Mac over
Bluetooth (the long ones over the Pocket's WiFi), and imports each as a meeting, which the meetings' pipeline
([meetings.md](meetings.md)) transcribes and summarizes here. The Pocket's
firmware stays as it is; nothing goes to the vendor.

The code: `core/src/pocket.rs` (the thread, settings, sync), `core/src/pocket/link.rs`
(Bluetooth, through btleplug), `core/src/pocket/wifi.rs` (the WiFi copy), `core/src/pocket/protocol.rs` (the commands and
answers, tested without a device), `app/src-tauri/src/pocket.rs` (the app's
commands and the `pocket` event), `app/src/lib/ui/PocketDialog.svelte` (the
dialog the sidebar's Pocket row opens), `app/src/lib/ui/PocketSettings.svelte`
(its page in Settings).

## The device and its protocol

The protocol is public, from captures of the vendor app:
[pocket-libre's PROTOCOL.md](https://github.com/shahcolate/pocket-libre/blob/main/PROTOCOL.md)
and [openpocket](https://github.com/OmiroAI/openpocket). What Yoru uses, as
seen on firmware 1.8.9:

- It advertises the 16-bit service `5536`; its name (`PKT01_<colour>_<id>`)
  shows only once connected. It takes one connection at a time and stops
  advertising during it, so the phone app and Yoru can't both hold it.
- Commands are ASCII, `APP&…`, written without response to
  `001120A3-2233-4455-6677-889912345678`; answers and events, `MCU&…`, come as
  notifications on the same characteristic, sometimes several in one
  notification. Recordings arrive on `001120A1-…` (notify).
- Every connection starts with `APP&SK&<16 hex digits>`, the session key.
  Without it the device ignores everything; a wrong key gets `MCU&SK&ERR` and
  a disconnect. After a hardware reset the device keeps the **first** key it
  hears (answering `MCU&SK&OK`) and drops that connection about a second
  later; that is the pairing.
- `BAT`, `FW`, `WF`, `SPACE` (free and total MB), `STE` (0 idle, 1 recording)
  for the status; `T&<UTC time>` sets its clock; `LIST_DIRS` then `LIST&<date>`
  for the recordings (`F&<date>&<name>&<seconds>`, closed by a count, so an
  incomplete listing is told from an empty one); `U&<date>&<name>` downloads
  one (`MCU&U&<size>`, the MP3 on the audio characteristic, `MCU&OFF`);
  `D&<date>&<name>` deletes one.
- While connected, `MCU&STA&<name>` and `MCU&STO` say a recording started and
  stopped. Yoru then lists and copies it.
- Recordings are MP3, 16 kHz mono, 32 kbps (4000 bytes a second), named after
  the UTC time they started (`20261119143012`; some `PH` and a shorter time).
  At that rate the encoder keeps nothing above about 4.5 kHz, which costs the
  transcription its consonants (measured on 2026-11-20); Yoru keeps the MP3
  as it is, without re-encoding it (until 2026-11-20 it became a 24 kbps AAC).
  Over Bluetooth to a Mac about 55 KB/s: an hour in about four and a half
  minutes.
- WiFi (pocket-libre's captures, firmware 1.7 and 1.8): `WIFIO` raises a
  hidden access point of the Pocket's own (`MCU&WIFIO`), `WIFI` gives its
  name and password (`MCU&WIFI&<ssid>&<password>`; the password is the
  session key's first eight characters), `WIFIS` its state (0 off, 3 coming
  up, 2 waiting, 1 a client joined), `WIFIC` turns it off. The file comes as
  raw bytes on TCP `192.168.200.1:8475`: open the socket, `U&<date>&<name>`
  for the size, `U&WIFI` (never without the socket open: the Pocket hangs
  and shuts down), then exactly that many bytes, ten fixed ones after them.
  The socket takes one or two connections per WiFi boot. About 1 MB/s.

Not used: USB mass storage, starting and stopping a
recording from the Mac (`APP&STA`/`APP&STO` aren't confirmed to do that), and
firmware updates.

## What Yoru does

A thread of its own (`Pocket::start`), with a tokio runtime:

1. With no key it waits (phase `unpaired`) and doesn't touch Bluetooth, so
   macOS asks for the Bluetooth permission only when the user pairs.
2. Pairing (`pocket_pair`): a new random key, sent to the first Pocket found;
   on `SK&OK` it goes to the keychain and Yoru reconnects.
3. Otherwise it scans almost without a break (30 s, a second's rest, again;
   until 2026-11-20 it rested 20 s, so finding it took up to 40 s). Found, it connects,
   sends the key, reads battery, firmware, storage and state, sets the clock
   (a setting), lists the recordings and copies the new ones, oldest first.
4. It stays connected: the Pocket drops a link that's quiet for 30 s, so
   its state is asked every 15 s (since 2026-11-20; before, the link broke
   every half minute and a recording waited for the next connection); a
   recording that stops is copied at once; battery and storage are read
   again every five minutes; the app's commands (sync now,
   copy these, delete one, settings, forget) are handled in between.
5. A recording of 15 minutes or more is copied over WiFi
   (`core/src/pocket/wifi.rs`) when the setting is on, the Pocket said its
   WiFi firmware and has more than 10% battery: `WIFIC` then `WIFIO`, the
   network's name, the Mac joins it (`networksetup -setairportnetwork`, Wi-Fi
   turned on first if it was off) and waits for an address in
   `192.168.200.x`, the socket, `U`, `U&WIFI`, the bytes; then, whatever
   happened, `WIFIC`, the Pocket's network removed from the Mac's list (this
   may need an administrator: harmless) and the Mac's Wi-Fi turned off and on,
   so macOS joins its own network again (or left off, as it was). The Mac is
   off its network for the copy: about half a minute plus a second per MB.
   If the Pocket doesn't turn WiFi on, or the Mac has no Wi-Fi, WiFi is left
   for the rest of the run; any other failure leaves it for the connection.
   Either way that recording is copied over Bluetooth at once. Shorter
   recordings always go over Bluetooth: setting WiFi up costs more than it
   saves. Not yet tried on the user's Pocket (firmware 1.8.9, WiFi V12).
6. A link that breaks is looked for again a second later. A download that
   ends short or stalls 15 s is skipped and tried again at the next
   connection, three times in all; after that, only when the user syncs.

A copied recording is checked against the size the device announced, written
to `pocket/recordings/<name>.mp3`, imported with `meetings::import_recording`
(the meeting's start is the recording's, its track's source "Pocket", its
`origin` `pocket`), and the copy removed. `pocket/synced.json` maps each
recording to its meeting, so a meeting deleted here doesn't come back while
the recording is still on the Pocket.

## Where things are

In Yoru's data folder (`~/Library/Application Support/yoru-hub/pocket/`):
`settings.json`, `synced.json`, `device.json` (the last Pocket's name). The
session key is in the keychain vault, account `pocket.session-key`. A key from
a pairing made before Yoru's, kept by `security` under the service
`pocket-ble-session-key`, is moved into the vault the first time.

## Settings

In Settings, under Pocket: the Pocket's name and state, with a button to its
dialog (Pair when it isn't paired), then:

- **Use the Pocket** (on): look for it and stay connected.
- **Sync on its own** (on): copy new recordings when the Pocket is near and
  when one ends; off, only on Sync.
- **Free the Pocket after copying** (off): delete a recording from the Pocket
  once it is a meeting.
- **Copy over WiFi when possible** (on): long recordings over the Pocket's
  WiFi, else Bluetooth (What Yoru does, 5).
- **Set the Pocket's clock** (on): at each connection, in UTC.
- **Forget this Pocket**: drop the key; pairing again needs a reset.

## The sidebar and the dialog

While the Pocket is on, the sidebar shows it above the board, in the same
kind of row: a dot on the left (green when connected, grey otherwise), its
name, Connected or Not connected, and the Bluetooth icon while connected.
A click opens its dialog, after the vendor app's
"device sync": the Pocket's name and connection,
battery, free storage and firmware; the new recordings, with select all and
"Sync N recordings"; the copy under way with its progress and how many wait;
"Everything is synced" when nothing is new; the recordings already in
Meetings, each with Open and Delete from the Pocket. Unpaired, the three steps
of the reset and the Pair button.

While a sync copies, Home lists the recordings being copied at the top
(From the Pocket): the one under way with its percentage (and "over WiFi"
when it is), then those waiting, from the status's `transfer` (`name`,
`bytes`, `total`, `wifi` and the names still `waiting`). When one becomes
a summary on its own, the notch says so (docs/notch.md, A summary ready).

The app needs `NSBluetoothAlwaysUsageDescription` in `Info.plist`; a binary
run outside the bundle (`cargo run`) doesn't start the Pocket's thread, since
macOS would end it.

## What Pocket's own app makes of a recording

Studied on 2026-11-20 from the web app (app.heypocket.com, which the Mac app loads) and its public configuration (`GET https://production.heypocketai.com/api/v1/configs`). It is the model for Yoru's summaries ([meetings.md](meetings.md), Summary); the prompts themselves are on Pocket's server and can't be read from the app, which only sends a template id, a model id and a language.

- **Summary**: one Markdown document written by the chosen model (Gemini 3 Flash by default; GPT-6, Claude Sonnet 5 and, with Pro, larger ones). It opens with a paragraph, then sections named for the content ("Key decisions", "Next steps", "Technical notes"), items with a bold subject. The model can put cards in it, `<pocket:chart type="bar" title="…">Label | value</pocket:chart>` and `<pocket:timeline title="…">date | title | description</pocket:timeline>` (also flowchart, decision tree, email and message), and LaTeX formulas.
- **Templates** ("themes"): 89, 8 of them free (Auto Detect, Meeting Notes, Call Notes, Interview Notes, Dictation Notes, Key Points, Lecture Notes, Dictation Format), the others by trade (medical SOAP, sales BANT, legal, education, minutes…). Each is an id, a title and a line ("Topic · Agreement · Conclusion"). Custom templates are sections with a title and instructions, plus whether to make to-dos and a mind map.
- **To-dos**: from the same pass, `{label, assignee, dueDate, status, type}` with `type` `create_reminder`, `send_message` or `draft_email` (the last two with the text to send); the due date is worked out from what was said. The To-dos page filters them by kind; each opens with its date, who, its source recording, Copy, Share and Mark as done.
- **Mind maps**: made on request, four kinds (overview, drivers, topics, people), as lists of nodes pointing to their parent.
- **The recording's page**: Notes (the summary, with More detail, Shorter and Rewrite, the only prompts in the client) and Source (the transcript, speakers to map, merge or bulk assign, markers); its menu has the transcript, history, regenerate (template, model, language), export, audio and feedback.
- **Ask Pocket**: a chat over the recordings with two tools, `pocket_memory` (search of recordings and transcripts, on Turbopuffer) and `web_search` (Exa); Thinking or Pro mode, files, deep research.
- **Developers**: API keys, webhooks and an MCP connector, a possible way for Yoru Hub to read what Pocket's cloud made, if the user ever uses it.
