<!--
  Settings > Intelligence: every model Yoru uses, one row per job (listens,
  thinks, speaks, transcribes, summarizes, remembers, searches) and the keys.
  A closed row says who does the job and whether the data leaves the Mac;
  an open one picks the service, then the model. A service without its key
  asks for it inside the row, and the job keeps its old service until the
  key works. Local models download from the row. Gemini Live hears and
  speaks, so it merges Listens and Speaks into one row. Each row also holds
  how its job goes: whether Yoru listens and answers aloud, follow-ups, and
  a meeting's transcript and summary. This is the only place for them.
-->
<script module lang="ts">
  export type Job = "hear" | "think" | "speak" | "transcribe" | "summarize" | "memory" | "web" | "keys";
</script>

<script lang="ts">
  import { untrack } from "svelte";
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { look } from "../look.svelte";
  import { store } from "../store.svelte";
  import { profileDefaults, PROFILES, type ProfileId } from "../profile-defaults";
  import type { AccountVoice, AssistantSettings, Brain, Effort, KeySource, MeetingSettings, ModelInfo, Summarizers, VoiceInfo, VoiceSettings } from "../types";
  import type { IconName } from "../icons/icons";
  import Icon from "./Icon.svelte";
  import KeyField from "./KeyField.svelte";
  import ModelPicker from "./ModelPicker.svelte";
  import Toggle from "./Toggle.svelte";

  let { row = null }: { row?: Job | null } = $props();

  const t = $derived(i18n.t.intelligence);
  const tb = $derived(i18n.t.board);
  const ts = $derived(i18n.t.settings);
  const tm = $derived(i18n.t.meetings);
  const s = $derived(store.info?.settings);
  const brain = $derived(store.info?.brain ?? null);

  // ---- What the backend knows ----

  let voice = $state<VoiceInfo | null>(null);
  let meet = $state<MeetingSettings | null>(null);
  /** Whether Parakeet is downloaded; null until known. */
  let parakeet = $state<boolean | null>(null);
  let engines = $state<Summarizers | null>(null);
  let ollama = $state<string[] | null>(null);
  let ollamaError = $state("");
  let account = $state<AccountVoice[]>([]);
  let error = $state("");

  const readVoice = async () => (voice = await call<VoiceInfo>("voice_info").catch(() => voice));
  async function readMeetings() {
    [meet, parakeet, engines] = await Promise.all([
      call<MeetingSettings>("meeting_settings").catch(() => meet),
      call<boolean>("meeting_model").catch(() => parakeet),
      call<Summarizers>("meeting_summarizers").catch(() => engines),
    ]);
  }
  async function readOllama() {
    ollamaError = "";
    try {
      ollama = await call<string[]>("ollama_models");
    } catch (e) {
      ollama = null;
      ollamaError = why(e);
    }
  }
  $effect(() => {
    void readVoice();
    void readMeetings();
  });
  // While a voice model downloads or Piper installs, follow it.
  $effect(() => {
    if (!voice?.download && !voice?.piper_install && voice?.stt !== "starting" && voice?.tts !== "starting") return;
    const id = setInterval(readVoice, 700);
    return () => clearInterval(id);
  });
  // ElevenLabs' account voices aren't in the catalog: asked once its key is there.
  const elevenKey = $derived(!!voice?.providers.find((p) => p.id === "elevenlabs")?.key);
  $effect(() => {
    if (!elevenKey) return void (account = []);
    call<AccountVoice[]>("elevenlabs_voices").then((v) => (account = v), () => {});
  });

  async function guard(job: () => Promise<unknown>) {
    error = "";
    try {
      await job();
    } catch (e) {
      error = why(e);
    }
  }
  const setVoice = (change: Partial<VoiceSettings>) =>
    guard(async () => {
      if (voice) voice = await call<VoiceInfo>("set_voice", { settings: { ...$state.snapshot(voice.settings), ...change } });
    });
  const setMeet = (change: Partial<MeetingSettings>) =>
    guard(async () => {
      if (meet) meet = await call<MeetingSettings>("set_meeting_settings", { settings: { ...meet, ...change } });
    });
  const setBrain = (change: (x: AssistantSettings) => void) => guard(() => store.saveSettings(change));

  // ---- Services ----

  const LOCAL = "local";
  const SAME = "same";
  const BRAINS: Brain[] = ["anthropic", "openai", "google", "groq", "ollama"];
  const LANGS = ["it", "en"];
  /** A service's logo: a letter on its colour. */
  const LOGOS: Record<string, [string, string]> = {
    local: ["⌘", "#6e6890"],
    ollama: ["O", "#4b4b4b"],
    anthropic: ["A", "#c96442"],
    openai: ["O", "#10a37f"],
    google: ["G", "#4285f4"],
    groq: ["G", "#f55036"],
    elevenlabs: ["11", "#2a2a2a"],
    tavily: ["T", "#3b6df0"],
  };
  const PLACEHOLDERS: Record<string, string> = { anthropic: "sk-ant-…", openai: "sk-proj-…", google: "AIza…", groq: "gsk_…", tavily: "tvly-…" };
  const serviceOf = (id: string | undefined | null) => (!id ? null : id.includes(":") ? id.slice(0, id.indexOf(":")) : LOCAL);
  const isLocal = (svc: string | null) => svc === LOCAL || svc === "ollama";
  function nameOf(svc: string): string {
    if (svc === LOCAL) return t.thisMac;
    if (svc === SAME) return t.sameAsThink;
    const fixed: Record<string, string> = { anthropic: "Anthropic", openai: "OpenAI", google: "Google", groq: "Groq", ollama: "Ollama", tavily: "Tavily" };
    return fixed[svc] ?? voice?.providers.find((p) => p.id === svc)?.name ?? svc;
  }
  function keyOf(svc: string): KeySource | null {
    const i = store.info;
    const of: Record<string, KeySource | null | undefined> = { anthropic: i?.key, openai: i?.openai_key, google: i?.google_key, groq: i?.groq_key, tavily: i?.tavily_key };
    return svc in of ? (of[svc] ?? null) : (voice?.providers.find((p) => p.id === svc)?.key ?? null);
  }
  const needsKey = (svc: string) => svc !== LOCAL && svc !== "ollama" && svc !== SAME && !keyOf(svc);
  const uniq = (ids: (string | null)[]) => [...new Set(ids.filter((x): x is string => !!x))].sort((a, b) => (a === LOCAL ? -1 : b === LOCAL ? 1 : 0));
  const profile = (svc: string) => ((PROFILES as string[]).includes(svc) ? profileDefaults(svc as ProfileId, voice?.recommendation?.whisper ?? "small") : null);

  // ---- Voice models ----

  const vs = $derived(voice?.settings);
  const live = $derived(!!vs?.live);
  const whispers = $derived(voice?.models.filter((m) => m.kind === "whisper") ?? []);
  // `elevenlabs:voice` only stands for the account's voices, listed apart.
  const voiceModels = $derived(voice?.models.filter((m) => m.kind === "voice" && m.id !== "elevenlabs:voice") ?? []);
  const model = (id: string | undefined) => voice?.models.find((m) => m.id === id);
  const missing = (m: ModelInfo | undefined) => !!m && !m.installed && !m.provider;
  const size = (m: ModelInfo) => (m.bytes >= 1e9 ? `${(m.bytes / 1e9).toFixed(1).replace(".", i18n.lang === "it" ? "," : ".")} GB` : `${Math.round(m.bytes / 1e6)} MB`);
  const ACCOUNT = "elevenlabs:voice/";
  /** A service's voices for a language: the catalog's, and for ElevenLabs the account's. */
  function voicesIn(svc: string, lang: string): { id: string; label: string }[] {
    const list = voiceModels
      .filter((m) => serviceOf(m.id) === svc && (m.language === lang || !m.language))
      .map((m) => ({ id: m.id, label: voiceName(m.id) + (m.note ? ` — ${m.note}` : "") + (missing(m) ? ` · ${t.toDownload(size(m))}` : "") }));
    if (svc === "elevenlabs") list.push(...account.map((a) => ({ id: `${ACCOUNT}${a.voice_id}`, label: `${a.name}${a.category ? ` · ${a.category}` : ""}` })));
    return list;
  }
  /** A voice's short name: Piper's it_IT-paola-medium is Paola. */
  function voiceName(id: string | undefined): string {
    if (!id) return "—";
    const piper = /^[a-z]{2}_[A-Z]{2}-([a-z]+)-/.exec(id);
    if (piper) return piper[1][0].toUpperCase() + piper[1].slice(1);
    if (id.startsWith(ACCOUNT)) return account.find((a) => `${ACCOUNT}${a.voice_id}` === id)?.name ?? id.slice(ACCOUNT.length);
    const note = model(id)?.note;
    return note?.split(/[,—·]/)[0].trim() || id.replace(/^[^:]+:/, "");
  }
  const whisperName = (id: string) => (serviceOf(id) === LOCAL ? `Whisper ${id}` : id.replace(/^[^:]+:/, ""));
  const hearing = $derived(uniq([...whispers.map((m) => serviceOf(m.id)), voice?.live_voices.length ? "google" : null]));
  const speaking = $derived(uniq([...voiceModels.map((m) => serviceOf(m.id)), voice?.providers.some((p) => p.id === "elevenlabs") ? "elevenlabs" : null]));
  const spoken = $derived(vs ? serviceOf(vs.voices[i18n.lang] ?? Object.values(vs.voices).find(Boolean)) : null);
  const intents = $derived(voice?.models.filter((m) => m.kind === "intent") ?? []);
  const turns = $derived(voice?.models.filter((m) => m.kind === "turn") ?? []);
  const dl = $derived(voice?.download ?? null);

  // ---- The brain's models ----

  const MODEL_FIELD = { anthropic: "model", openai: "openai_model", google: "google_model", groq: "groq_model", ollama: "ollama_model" } as const;
  const modelOf = (b: Brain) => (s ? s[MODEL_FIELD[b]] : "");
  const catalog = (b: Brain) => (b === "openai" ? store.info?.openai_models : b === "google" ? store.info?.google_models : b === "groq" ? store.info?.groq_models : undefined) ?? [];
  const CLAUDE = $derived([
    ["claude-opus-5-5", ts.claudeModels.opus],
    ["claude-sonnet-5-5", ts.claudeModels.sonnet],
    ["claude-haiku-5-5", ts.claudeModels.haiku],
  ] as const);
  const EFFORTS: Effort[] = ["low", "medium", "high", "xhigh", "max"];
  $effect(() => {
    if (brain === "ollama" || pending.think === "ollama") void readOllama();
  });

  // ---- Jobs ----

  type Option = { id: string; name: string; note: string; recommended?: boolean; download?: string; progress?: number | null };
  const ICONS: Record<Job, IconName> = { hear: "mic", think: "message-circle", speak: "audio-lines", transcribe: "file-text", summarize: "sparkles", memory: "brain", web: "search", keys: "key-round" };
  const GROUPS = $derived<{ id: string; jobs: Job[] }[]>([
    { id: "talk", jobs: live ? ["hear", "think"] : ["hear", "think", "speak"] },
    { id: "meetings", jobs: ["transcribe", "summarize"] },
    { id: "always", jobs: ["memory", "web", "keys"] },
  ]);

  let open = $state<Job | null>(untrack(() => row) ?? "think");
  /** A service picked whose key is still missing: the job keeps its old one meanwhile. */
  let pending = $state<Partial<Record<Job, string>>>({});
  /** Whose every model is listed: a row's "All models" link. */
  let more = $state<Job | null>(null);
  /** After a key works, the other jobs its service can do. */
  let suggest = $state<{ svc: string; jobs: Job[] } | null>(null);
  /** The keys row: which service's field is open. */
  let keyOpen = $state<string | null>(null);

  const toggle = (job: Job) => ((open = open === job ? null : job), (more = null), (suggest = null));

  /** The service doing a job now. */
  function current(job: Job): string | null {
    if (job === "hear") return !vs ? null : live ? "google" : serviceOf(vs.model);
    if (job === "think" || job === "memory") return brain;
    if (job === "speak") return spoken;
    if (job === "transcribe") return !meet ? null : meet.transcribe_by === "local" ? LOCAL : serviceOf(meet.transcribe_by);
    if (job === "summarize") return !meet ? null : meet.summarize_by ? (engines?.models.find((m) => m.id === meet!.summarize_by)?.provider ?? serviceOf(meet.summarize_by)) : SAME;
    if (job === "web") return brain === "anthropic" && s?.web_search ? "anthropic" : keyOf("tavily") ? "tavily" : null;
    return null;
  }
  const shown = (job: Job) => pending[job] ?? current(job);

  /** The services that can do a job. */
  function services(job: Job): string[] {
    if (job === "hear") return hearing;
    if (job === "think") return BRAINS;
    if (job === "speak") return speaking;
    if (job === "transcribe") return uniq((engines?.transcribers ?? []).map((x) => (x.local ? LOCAL : serviceOf(x.id))));
    if (job === "summarize") return [SAME, ...uniq((engines?.models ?? []).map((m) => m.provider))];
    return [];
  }

  /** A job's models on a service, the recommended first among equals. */
  function options(job: Job, svc: string): Option[] {
    if (job === "hear" && svc === "google")
      return (voice?.live_voices ?? []).map(([name, desc]) => ({ id: name, name: `Gemini Live · ${name}`, note: tb.liveStyles[desc.toLowerCase()] ?? desc, recommended: name === "Puck" }));
    if (job === "hear")
      return whispers
        .filter((m) => serviceOf(m.id) === svc)
        .map((m) => ({
          id: m.id,
          name: whisperName(m.id),
          note: m.note,
          recommended: m.id === (svc === LOCAL ? voice?.recommendation?.whisper : profile(svc)?.hearing),
          download: missing(m) ? size(m) : undefined,
          progress: dl?.[0] === m.id ? dl[1] : null,
        }));
    if (job === "think") {
      const b = svc as Brain;
      const rec = profile(svc)?.brainModel;
      const list: Option[] =
        b === "anthropic"
          ? CLAUDE.map(([id, label]) => ({ id, name: label.split(" — ")[0], note: label.split(" — ").slice(1).join(" — "), recommended: id === rec }))
          : b === "ollama"
            ? (ollama ?? []).map((m) => ({ id: m, name: m, note: "" }))
            : catalog(b).map((m) => ({ id: m.id, name: m.note.split(":")[0] || m.id, note: m.note.split(":").slice(1).join(":").trim(), recommended: m.id === rec }));
      // A model typed or picked from the whole list is shown too.
      const now = brain === b ? modelOf(b) : "";
      if (now && !list.some((o) => o.id === now)) list.unshift({ id: now, name: now, note: "" });
      return list;
    }
    if (job === "transcribe")
      return (engines?.transcribers ?? [])
        .filter((x) => (x.local ? LOCAL : serviceOf(x.id)) === svc)
        .map((x) => {
          const [name, note] = x.local ? [tm.parakeet.replace(/^.*\((.*)\)$/, "$1"), tm.simpleLocal] : (tm.transcribers[x.id] ?? [x.id, ""]);
          return {
            id: x.id,
            name,
            note: x.detailed ? note : `${note} · ${t.simpleOnly}`,
            recommended: x.id === (svc === LOCAL ? "local" : svc === "google" ? "google:gemini-3.8-flash" : svc === "groq" ? "groq:whisper-large-v3-turbo" : ""),
            download: x.local && parakeet === false && store.modelDownload === null ? "" : undefined,
            progress: x.local && store.modelDownload !== null ? Math.round(store.modelDownload * 100) : null,
          };
        });
    if (job === "summarize")
      return (engines?.models ?? []).filter((m) => m.provider === svc).map((m) => ({ id: m.id, name: m.note.split(":")[0] || m.model, note: m.note.split(":").slice(1).join(":").trim() }));
    return [];
  }

  /** The model chosen for a job, as its options name it. */
  function value(job: Job): string {
    if (job === "hear") return vs ? vs.live || vs.model : "";
    if (job === "think") return brain ? modelOf(brain) : "";
    if (job === "transcribe") return meet?.transcribe_by ?? "";
    if (job === "summarize") return meet?.summarize_by ?? "";
    return "";
  }
  const optionName = (job: Job) => {
    const svc = current(job);
    return (svc && options(job, svc).find((o) => o.id === value(job))?.name) || value(job);
  };
  /** The option to take on a service: the recommended one if it's ready, else the first ready. */
  function best(job: Job, svc: string): Option | undefined {
    const list = options(job, svc);
    return list.find((o) => o.recommended && o.download === undefined) ?? list.find((o) => o.download === undefined) ?? list[0];
  }

  function choose(job: Job, svc: string) {
    suggest = null;
    more = null;
    if (needsKey(svc)) return void (pending = { ...pending, [job]: svc });
    const { [job]: _, ...rest } = pending;
    pending = rest;
    if (job === "hear") {
      if (svc === "google") return setVoice({ live: vs?.live || best(job, svc)?.id || "Puck" });
      const keep = serviceOf(vs?.model) === svc;
      return setVoice({ live: "", enabled: true, model: keep ? vs!.model : (best(job, svc)?.id ?? vs!.model) });
    }
    if (job === "think") {
      const b = svc as Brain;
      return setBrain((x) => {
        x.provider = b;
        if (!x[MODEL_FIELD[b]]) x[MODEL_FIELD[b]] = profile(b)?.brainModel ?? "";
      });
    }
    if (job === "speak") return speakWith(svc);
    if (job === "transcribe") {
      const o = best(job, svc);
      if (o) return pickTranscriber(o.id);
    }
    if (job === "summarize") return setMeet({ summarize_by: svc === SAME ? "" : (best(job, svc)?.id ?? "") });
  }

  async function pick(job: Job, svc: string, id: string) {
    if (job === "hear" && svc === "google") return setVoice({ live: id });
    if (job === "hear") {
      if (missing(model(id))) await guard(() => call("download_model", { id }));
      await readVoice();
      if (!missing(model(id))) await setVoice({ live: "", enabled: true, model: id });
      return;
    }
    if (job === "think") return setBrain((x) => ((x.provider = svc as Brain), (x[MODEL_FIELD[svc as Brain]] = id)));
    if (job === "transcribe") return pickTranscriber(id);
    if (job === "summarize") return setMeet({ summarize_by: id });
  }

  /** Parakeet downloads first; a transcriber that doesn't time its lines makes the transcript simple. */
  async function pickTranscriber(id: string) {
    if (id === "local" && parakeet === false) {
      store.modelDownload = 0;
      await guard(() => call("meeting_model_download"));
      store.modelDownload = null;
      parakeet = await call<boolean>("meeting_model").catch(() => parakeet);
      if (!parakeet) return;
    }
    const detailed = engines?.transcribers.find((x) => x.id === id)?.detailed;
    await setMeet({ transcribe_by: id, ...(detailed === false ? { detail: "simple" as const } : {}) });
  }

  // Each language takes the service's first voice for it, unless it has one there already.
  function speakWith(svc: string) {
    if (!vs) return;
    const next = { ...vs.voices };
    for (const l of LANGS) {
      const list = voicesIn(svc, l);
      if (list.length && serviceOf(next[l]) !== svc) next[l] = list[0].id;
    }
    return setVoice({ voices: next });
  }
  // Answering aloud again keeps the voices picked before; with none yet, this Mac's first for each language.
  function speakOn() {
    if (!vs) return;
    if (Object.values(vs.voices).some(Boolean)) return setVoice({ speak: true });
    const voices = { ...vs.voices };
    for (const l of LANGS) {
      const first = voiceModels.find((m) => serviceOf(m.id) === LOCAL && m.language === l);
      if (first) voices[l] = first.id;
    }
    return setVoice({ speak: true, voices });
  }

  // ---- How meetings go ----

  const simple = $derived(meet?.detail === "simple");
  /** The transcriber chosen, as the app offers it. */
  const by = $derived(engines?.transcribers.find((x) => x.id === meet?.transcribe_by));
  /** Switch the kind of transcript; a detailed one needs a transcriber that times its lines. */
  function setDetail(detail: MeetingSettings["detail"]) {
    if (!meet || meet.detail === detail) return;
    if (detail === "detailed" && by && !by.detailed) setMeet({ detail, transcribe_by: "local" });
    else setMeet({ detail });
  }
  const MEET_LANGS = ["", "it", "en", "es", "fr", "de", "pt"];
  const WHEN = $derived([
    ["after", tm.whenAfter],
    ["manual", tm.whenManual],
    ["live", tm.whenLive],
  ] as const);

  async function getVoice(id: string) {
    await guard(() => call("download_model", { id }));
    await readVoice();
  }
  async function installPiper() {
    const job = guard(() => call("install_engine", { engine: "piper" }));
    setTimeout(readVoice, 200);
    await job;
    await readVoice();
  }
  const canTry = $derived(voice?.tts === "ready" && store.boardState === "live");
  let trying = $state("");
  async function tryVoice(lang: string) {
    trying = lang;
    await guard(() => call("test_voice", { board: null, language: lang }));
    trying = "";
  }

  /** A key saved: read everything again, use the service the row was waiting for, and offer it elsewhere. */
  async function keySaved(job: Job | null, svc: string) {
    await Promise.all([store.refreshInfo().catch(() => {}), readVoice(), readMeetings()]);
    if (needsKey(svc)) return;
    if (job && pending[job] === svc) choose(job, svc);
    const others = (["hear", "think", "transcribe", "summarize", "speak"] as Job[]).filter((j) => j !== job && services(j).includes(svc) && current(j) !== svc && !(j === "speak" && live));
    suggest = others.length ? { svc, jobs: others } : null;
  }

  // ---- What a closed row says ----

  function chip(job: Job): { text: string; svc: string | null; kind?: "todo" | "linked" } {
    if (job === "think" && !brain) return { text: t.choose, svc: null, kind: "todo" };
    if (job === "memory") return { text: t.sameAsThink, svc: null, kind: "linked" };
    if (job === "keys") {
      const n = keys.filter((k) => k.source).length;
      return { text: n ? t.services(n) : t.none, svc: null, kind: "linked" };
    }
    if (job === "web") {
      const w = current("web");
      return w ? { text: w === "anthropic" ? t.claudeSearch : "Tavily", svc: w } : { text: t.webOff, svc: null, kind: "linked" };
    }
    if (job === "summarize" && current(job) === SAME) return { text: t.sameAsThink, svc: null, kind: "linked" };
    if (job === "speak") return { text: LANGS.map((l) => voiceName(vs?.voices[l])).filter((x) => x !== "—").join(" · ") || "—", svc: spoken };
    if (job === "hear" && live) return { text: `Gemini Live · ${vs!.live}`, svc: "google" };
    if (job === "hear") return { text: whisperName(vs?.model ?? ""), svc: current(job) };
    return { text: optionName(job), svc: current(job) };
  }
  /** Whether a job's data stays on this Mac; null when it doesn't say. */
  function stays(job: Job): boolean | null {
    if (job === "keys") return null;
    if (job === "summarize" && current(job) === SAME) return brain ? isLocal(brain) : null;
    if (job === "web") return current("web") === null;
    const svc = current(job === "memory" ? "think" : job);
    return svc ? isLocal(svc) : null;
  }
  function subtitle(job: Job): { text: string; warn?: boolean } {
    if (job === "think" && store.info?.limited) return { text: store.info.limited, warn: true };
    if (job === "think" && !brain) return { text: t.firstRun };
    if (job === "hear" && dl && whispers.some((m) => m.id === dl[0])) return { text: t.downloading(whisperName(dl[0]), Math.round(dl[1])) };
    if (job === "transcribe" && store.modelDownload !== null) return { text: t.downloading("Parakeet", Math.round(store.modelDownload * 100)) };
    if (job === "hear" && live) return { text: t.liveSays };
    if (job === "hear" && vs && !vs.enabled) return { text: t.hearOff, warn: true };
    if (job === "speak" && vs && !vs.speak) return { text: t.speakOff, warn: true };
    if (job === "speak" && dl && voiceModels.some((m) => m.id === dl[0])) return { text: t.downloading(voiceName(dl[0]), Math.round(dl[1])) };
    return { text: t.says[job] };
  }
  /** Where the job's data goes, at the bottom of its open row. */
  function footer(job: Job): { local: boolean; text: string } | null {
    const local = stays(job);
    if (local === null) return null;
    if (job === "web") return local ? { local, text: t.webStays } : { local, text: t.webGoes(nameOf(current("web")!)) };
    const what = t.what[job === "memory" ? "think" : job];
    const svc = job === "summarize" && current(job) === SAME ? brain : current(job === "memory" ? "think" : job);
    return local ? { local, text: t.stays(what) } : { local, text: t.goes(what, nameOf(svc ?? "")) };
  }

  // ---- The keys ----

  type Key = { id: string; name: string; source: KeySource | null; env: string };
  const keys = $derived.by<Key[]>(() => {
    const brainKeys = ["anthropic", "openai", "google", "groq"].map((id) => ({ id, name: nameOf(id), source: keyOf(id), env: voice?.providers.find((p) => p.id === id)?.key_env ?? "" }));
    const voices = (voice?.providers ?? [])
      .filter((p) => !["anthropic", "openai", "google", "groq", "ollama", "tavily"].includes(p.id))
      .map((p) => ({ id: p.id, name: p.name, source: p.key, env: p.key_env }));
    return [...brainKeys, ...voices, { id: "tavily", name: "Tavily", source: keyOf("tavily"), env: "TAVILY_API_KEY" }];
  });
  const jobName = (job: Job) => (job === "hear" && live ? t.jobs.hearLive : t.jobs[job]);
</script>

{#each GROUPS as g (g.id)}
  <h2 class="section-title">{t.groups[g.id]}</h2>
  <div class="panel acc">
    {#each g.jobs as job (job)}
      {@const c = chip(job)}
      {@const sub = subtitle(job)}
      {@const local = stays(job)}
      <div class="ar" class:open={open === job}>
        <button class="ar-h" aria-expanded={open === job} onclick={() => toggle(job)}>
          <span class="ic"><Icon name={ICONS[job]} size={17} /></span>
          <span class="w">
            <b>{jobName(job)}{#if job === "think" && store.info?.limited}<span class="dot wait"></span>{/if}</b>
            <span class:warn={sub.warn} title={sub.text}>{sub.text}</span>
          </span>
          <span class="right">
            <span class="who {c.kind ?? ''}">
              {#if c.kind === "linked" && job === "memory"}<Icon name="lock" size={12} />{/if}
              {#if c.svc && !c.kind}{@render logo(c.svc)}{/if}
              <span class="who-t">{c.text}</span>
            </span>
            {#if local !== null}
              <span class="loc" class:mac={local} title={local ? t.onMac : t.leaves}><Icon name={local ? "laptop" : "wifi"} size={13} stroke={2} /></span>
            {/if}
          </span>
          <span class="chev"><Icon name="chevron-right" size={16} /></span>
        </button>
        {#if open === job}
          <div class="ar-b">{@render body(job)}</div>
        {/if}
      </div>
    {/each}
  </div>
{/each}
{#if error}<p class="error-text note">{error}</p>{/if}

{#snippet logo(svc: string)}
  {@const l = LOGOS[svc] ?? [nameOf(svc).slice(0, 1).toUpperCase(), "#555"]}
  <span class="logo" style:background={l[1]}>{l[0]}</span>
{/snippet}

{#snippet body(job: Job)}
  {#if job === "memory"}
    <div class="lock">
      <span><Icon name="lock" size={14} /> {t.memoryLock(brain ? optionName("think") : "")}</span>
      <button class="btn link" onclick={() => store.go("memory")}>{t.openMemory} ›</button>
    </div>
  {:else if job === "keys"}
    <div class="list">
      {#each keys as k (k.id)}
        <div class="keyline">
          {@render logo(k.id)}
          <span class="kname">{k.name} <span class="dim small">{k.source === "env" ? t.fromEnv(k.env) : k.source ? t.saved : t.noKey}</span></span>
          <button class="btn small" onclick={() => (keyOpen = keyOpen === k.id ? null : k.id)}>{keyOpen === k.id ? t.close : k.source ? t.manage : t.add}</button>
        </div>
        {#if keyOpen === k.id}
          <div class="keyfield"><KeyField provider={k.id} source={k.source} placeholder={PLACEHOLDERS[k.id]} onchange={() => keySaved(null, k.id)} /></div>
        {/if}
      {/each}
    </div>
    <p class="small dim">{t.keysNote}</p>
    {@render suggestion()}
  {:else if job === "web"}
    {#if brain === "anthropic" && s}
      <div class="line">
        <span class="t"><b>{t.claudeSearch}</b><span>{t.webClaudeHint}</span></span>
        <Toggle checked={s.web_search} label={t.claudeSearch} onchange={(on) => setBrain((x) => (x.web_search = on))} />
      </div>
    {/if}
    <div class="line col">
      <span class="t"><b>{@render logo("tavily")} Tavily</b><span>{t.tavilyHint}</span></span>
      <KeyField provider="tavily" source={keyOf("tavily")} placeholder={PLACEHOLDERS.tavily} onchange={() => keySaved("web", "tavily")} />
    </div>
    {@render foot(job)}
  {:else}
    {@const svc = shown(job)}
    {@render switches(job)}
    <div class="lbl">{t.who}</div>
    <div class="svcs" role="radiogroup" aria-label={t.who}>
      {#each services(job) as v (v)}
        <button class="svc" class:on={v === svc} role="radio" aria-checked={v === svc} onclick={() => choose(job, v)}>
          {#if v === SAME}<span class="logo same">=</span>{:else}{@render logo(v)}{/if}
          {job === "hear" && v === "google" ? tb.live : nameOf(v)}
          {#if needsKey(v)}<span class="k" title={t.needKey(nameOf(v))}><Icon name="key-round" size={11} /></span>{/if}
        </button>
      {/each}
    </div>
    {#if svc}<p class="svc-note">{t.notes[svc] ?? ""}</p>{/if}
    {#if svc && needsKey(svc)}
      <div class="keybox">
        <span><b>{t.needKey(nameOf(svc))}</b> {t.keyStays}{#if current(job)} {t.meanwhile(chip(job).text)}{/if}</span>
        <KeyField provider={svc} source={null} placeholder={PLACEHOLDERS[svc]} onchange={() => keySaved(job, svc)} />
      </div>
    {:else if svc}
      {@render models(job, svc)}
      {@render extras(job, svc)}
    {/if}
    {@render how(job)}
    {@render suggestion()}
    {@render foot(job)}
  {/if}
{/snippet}

<!-- A service's models for a job, as a list to pick from; "All models" opens the service's whole list. -->
{#snippet models(job: Job, svc: string)}
  {@const list = options(job, svc)}
  {@const now = current(job) === svc ? value(job) : null}
  {#if job === "think" && svc === "ollama" && (ollamaError || (ollama && !ollama.length))}
    <p class="error-text small">{ollamaError || ts.ollamaNone}</p>
  {/if}
  {#if list.length}
    <div class="lbl">{job === "hear" && svc === "google" ? t.liveVoice : t.model}</div>
    <div class="models" role="radiogroup" aria-label={t.model}>
      {#each list as o (o.id)}
        <button class="model" role="radio" aria-checked={o.id === now} disabled={o.progress != null} onclick={() => pick(job, svc, o.id)}>
          <span class="radio" class:on={o.id === now}></span>
          <span class="n">
            <b>
              {o.name}
              {#if o.recommended}<span class="badge">{t.recommended}</span>{/if}
              {#if o.download !== undefined && o.progress == null}<span class="badge size">{t.toDownload(o.download)}</span>{/if}
            </b>
            {#if o.note}<span>{o.note}</span>{/if}
          </span>
          {#if o.progress != null}<span class="prog"><i style:width="{o.progress}%"></i></span>{/if}
        </button>
      {/each}
      {#if job === "think" && svc !== "ollama"}
        {#if more === job}
          <div class="all">
            <ModelPicker value={brain === svc ? modelOf(svc as Brain) : ""} providers={[{ id: svc, name: nameOf(svc) }]} bare custom={svc === "anthropic"} label={t.allModels(nameOf(svc))} onchange={(v) => pick(job, svc, v)} />
          </div>
        {:else}
          <button class="more" onclick={() => (more = job)}>{t.allModels(nameOf(svc))} ›</button>
        {/if}
      {:else if job === "think"}
        <div class="row-btns">
          <button class="more" onclick={readOllama}>{ts.ollamaRefresh}</button>
          <button class="more" onclick={() => call("open_key_page", { provider: "ollama" })}>{t.ollamaMore} ↗</button>
        </div>
      {:else if job === "hear" && svc !== LOCAL && svc !== "google"}
        {#if more === job}
          <div class="all">
            <ModelPicker value={vs?.model ?? ""} providers={[{ id: svc, name: nameOf(svc) }]} need="stt" label={t.allModels(nameOf(svc))} onchange={(v) => pick(job, svc, v)} />
          </div>
        {:else}
          <button class="more" onclick={() => (more = job)}>{t.allModels(nameOf(svc))} ›</button>
        {/if}
      {/if}
    </div>
  {:else if job === "think" && svc === "ollama"}
    <div class="row-btns">
      <button class="btn small" onclick={readOllama}>{ts.ollamaRefresh}</button>
      <button class="btn small" onclick={() => call("open_key_page", { provider: "ollama" })}>{t.ollamaMore} ↗</button>
    </div>
  {/if}
{/snippet}

<!-- What a job adds beside its model: Claude's effort, Ollama's thinking, the voice per language, Gemini Live's model, a turn's details. -->
{#snippet extras(job: Job, svc: string)}
  {#if job === "think" && svc === "anthropic" && brain === "anthropic" && s}
    <div class="line">
      <span class="t"><b>{t.effort}</b><span>{t.effortHint}</span></span>
      <div class="seg" role="radiogroup" aria-label={t.effort}>
        {#each EFFORTS as e (e)}
          <button role="radio" aria-checked={s.effort === e} class:on={s.effort === e} onclick={() => setBrain((x) => (x.effort = e))}>{ts.efforts[e]}</button>
        {/each}
      </div>
    </div>
  {/if}
  {#if job === "think" && svc === "ollama" && brain === "ollama" && s}
    <div class="line">
      <span class="t"><b>{t.ollamaThink}</b><span>{t.ollamaThinkHint}</span></span>
      <Toggle checked={s.think} label={t.ollamaThink} onchange={(on) => setBrain((x) => (x.think = on))} />
    </div>
  {/if}
  {#if job === "speak" && vs}
    <div class="lbl">{t.voiceFor}</div>
    {#each LANGS as lang (lang)}
      {@const list = voicesIn(svc, lang)}
      {@const m = model(vs.voices[lang])}
      <div class="line">
        <span class="t"><b>{t.langs[lang as "it" | "en"]}</b></span>
        <span class="how">
          {#if m && missing(m)}
            {#if dl?.[0] === m.id}<span class="dim small">{tb.downloading(Math.round(dl[1]))}</span>
            {:else}<button class="btn primary small" disabled={!!dl} onclick={() => getVoice(m.id)}>{tb.download} · {size(m)}</button>{/if}
          {/if}
          <select value={serviceOf(vs.voices[lang]) === svc ? vs.voices[lang] : ""} onchange={(e) => setVoice({ voices: { ...vs.voices, [lang]: e.currentTarget.value } })} aria-label={tb.voiceFor(lang)}>
            {#if serviceOf(vs.voices[lang]) !== svc}<option value="" disabled>—</option>{/if}
            {#each list as v (v.id)}<option value={v.id}>{v.label}</option>{/each}
            {#if vs.voices[lang]?.startsWith(ACCOUNT) && svc === "elevenlabs" && !account.length}<option value={vs.voices[lang]}>{vs.voices[lang].slice(ACCOUNT.length)}</option>{/if}
          </select>
          {#if canTry && vs.speak}
            <button class="btn small" disabled={!!trying} onclick={() => tryVoice(lang)}><Icon name="play" size={13} />{t.try}</button>
          {/if}
        </span>
      </div>
    {/each}
    {#if svc === LOCAL && voice && !voice.piper}
      <div class="line">
        <span class="t"><b>{tb.localVoiceEngine}</b>{#if voice.piper_install}<span>{voice.piper_install}</span>{/if}</span>
        <button class="btn small" disabled={!!voice.piper_install} onclick={installPiper}>{tb.installPiper}</button>
      </div>
    {/if}
    {#if voice?.tts_error && vs.speak}<p class="error-text small">{voice.tts_error}</p>{/if}
  {/if}
  {#if job === "hear" && svc === "google" && live && vs}
    <div class="line">
      <span class="t"><b>{tb.liveModel}</b><span>{tb.liveModelHint}</span></span>
      <select value={vs.live_model ?? ""} onchange={(e) => setVoice({ live_model: e.currentTarget.value })} aria-label={tb.liveModel}>
        <option value="">{tb.liveModelAuto}</option>
        {#each voice?.live_models ?? [] as [id, note] (id)}<option value={id}>{tb.liveModels[id] ?? note.split(":")[0]}</option>{/each}
      </select>
    </div>
    <div class="callout think"><p>{t.liveNote}</p></div>
  {/if}
  {#if job === "hear" && voice?.stt_error && vs?.enabled && !live}<p class="error-text small">{voice.stt_error}</p>{/if}
  {#if job === "summarize" && svc === SAME}
    <p class="small dim">{t.sameNow(brain ? optionName("think") : t.noBrainYet)}</p>
  {/if}
  {#if job === "hear" && look.advanced && !live && vs}
    <div class="adv">
      <div class="lbl">{t.turnDetails}<span class="faint">{t.advancedOption}</span></div>
      {@render small(tb.turnModel, tb.turnHint, vs.turn_model, tb.turnPause, turns, (id) => setVoice({ turn_model: id }))}
      {@render small(tb.intentModel, tb.intentHint, vs.intent_model, tb.intentPhrases, intents, (id) => setVoice({ intent_model: id }), !voice?.llm_server ? tb.needsLlamaServer : "")}
    </div>
  {/if}
{/snippet}

<!-- One job a small local model can do, or the built-in way ("" picks it). -->
{#snippet small(name: string, hint: string, value: string, builtIn: string, list: ModelInfo[], choose: (id: string) => void, blocked = "")}
  {@const m = model(value)}
  <div class="line">
    <span class="t"><b>{name}</b><span>{blocked || hint}</span></span>
    <span class="how">
      {#if m && missing(m)}
        {#if dl?.[0] === m.id}<span class="dim small">{tb.downloading(Math.round(dl[1]))}</span>
        {:else}<button class="btn primary small" disabled={!!dl} onclick={() => getVoice(m.id)}>{tb.download} · {size(m)}</button>{/if}
      {/if}
      <select {value} onchange={(e) => choose(e.currentTarget.value)} aria-label={name}>
        <option value="">{builtIn}</option>
        {#each list as option (option.id)}<option value={option.id} disabled={!!blocked}>{option.id}{option.note ? ` — ${option.note}` : ""}</option>{/each}
      </select>
    </span>
  </div>
{/snippet}

<!-- A row's on/off, above its service: whether Yoru listens, whether he answers aloud. -->
{#snippet switches(job: Job)}
  {#if vs && job === "hear" && !live}
    <div class="line onoff">
      <span class="t"><b>{tb.listensToggle}</b><span>{vs.enabled ? tb.listenHint : tb.listensOffHint}</span></span>
      <Toggle checked={vs.enabled} label={tb.listensToggle} onchange={(on) => setVoice({ enabled: on })} />
    </div>
  {/if}
  {#if vs && job === "speak"}
    <div class="line onoff">
      <span class="t"><b>{tb.speaksToggle}</b><span>{vs.speak ? tb.speaksHint : tb.speaksOffHint}</span></span>
      <Toggle checked={vs.speak} label={tb.speaksToggle} onchange={(on) => (on ? speakOn() : setVoice({ speak: false }))} />
    </div>
  {/if}
{/snippet}

<!-- How a job goes beside who does it: a conversation's follow-ups, a meeting's transcript and summary. -->
{#snippet how(job: Job)}
  {#if vs && ((job === "speak" && vs.speak) || (job === "hear" && live))}
    <div class="opts">
      <div class="line">
        <span class="t"><b>{tb.followUp}</b></span>
        <Toggle checked={vs.follow_up} label={tb.followUp} onchange={(on) => setVoice({ follow_up: on })} />
      </div>
    </div>
  {/if}
  {#if meet && job === "transcribe"}
    <div class="opts">
      <div class="line">
        <span class="t"><b>{tm.detect}</b><span>{tm.detectHint}</span></span>
        <Toggle checked={meet.detect} label={tm.detect} onchange={(on) => setMeet({ detect: on })} />
      </div>
      <div class="line col">
        <span class="t"><b>{tm.detail}</b><span>{tm.detailHint}</span></span>
        <div class="kinds" role="radiogroup" aria-label={tm.detail}>
          <button class="kind" class:on={!simple} role="radio" aria-checked={!simple} title={by && !by.detailed ? tm.detailedNeedsTimes : undefined} onclick={() => setDetail("detailed")}>
            <b>{tm.detailed}</b>
            <span class="sample timed">
              {#each tm.detailedExample as [at, who, said] (at)}<span class="mono faint">{at}</span><span class="sw">{who}</span><span>{said}</span>{/each}
            </span>
            <span class="dim small">{tm.detailedHint}</span>
          </button>
          <button class="kind" class:on={simple} role="radio" aria-checked={simple} onclick={() => setDetail("simple")}>
            <b>{tm.simple}</b>
            <span class="sample">{tm.simpleExample}</span>
            <span class="dim small">{tm.simpleHint}</span>
          </button>
        </div>
      </div>
      <div class="line col">
        <span class="t"><b>{tm.when}</b><span>{tm.whenHint}</span></span>
        <div class="chips" role="radiogroup" aria-label={tm.when}>
          {#each WHEN as [id, label] (id)}
            <button class="chip" class:on={meet.transcribe === id} role="radio" aria-checked={meet.transcribe === id} disabled={id === "live"} title={id === "live" ? tm.liveSoon : undefined} onclick={() => setMeet({ transcribe: id })}>{label}</button>
          {/each}
        </div>
      </div>
      <div class="line">
        <span class="t"><b>{tm.language}</b><span>{tm.languageHint}</span></span>
        <select value={meet.language} onchange={(e) => setMeet({ language: e.currentTarget.value })} aria-label={tm.language}>
          {#each MEET_LANGS as code (code)}<option value={code}>{code ? tm.languages[code as keyof typeof tm.languages] : tm.languageAuto}</option>{/each}
        </select>
      </div>
      <div class="line" class:off={simple || (by && !by.speakers)}>
        <span class="t"><b>{tm.speakers}</b><span>{by && !by.speakers ? tm.speakersNone : tm.speakersHint}</span></span>
        <Toggle checked={meet.speakers} label={tm.speakers} onchange={(on) => setMeet({ speakers: on })} />
      </div>
    </div>
  {/if}
  {#if meet && job === "summarize"}
    <div class="opts">
      <div class="line">
        <span class="t"><b>{tm.style}</b><span>{tm.styleHint}</span></span>
        <select value={meet.summary_style || "auto"} onchange={(e) => setMeet({ summary_style: e.currentTarget.value })} aria-label={tm.style}>
          {#each Object.entries(tm.styles) as [id, label] (id)}<option value={id}>{label}</option>{/each}
        </select>
      </div>
      <div class="line">
        <span class="t"><b>{tm.autoSummary}</b><span>{tm.autoSummaryHint}</span></span>
        <Toggle checked={meet.auto_summary} label={tm.autoSummary} onchange={(on) => setMeet({ auto_summary: on })} />
      </div>
    </div>
  {/if}
{/snippet}

<!-- After a key works: the other jobs its service could do. -->
{#snippet suggestion()}
  {#if suggest}
    {@const sg = suggest}
    <div class="callout think">
      <p>{t.suggest(nameOf(sg.svc))}</p>
      <span class="row-btns">
        {#each sg.jobs as j (j)}<button class="btn small" onclick={() => ((suggest = { ...sg, jobs: sg.jobs.filter((x) => x !== j) }), choose(j, sg.svc))}>{jobName(j)}</button>{/each}
        <button class="btn small quiet" onclick={() => (suggest = null)}>{t.dismiss}</button>
      </span>
    </div>
  {/if}
{/snippet}

{#snippet foot(job: Job)}
  {@const f = footer(job)}
  {#if f}
    <div class="foot" class:mac={f.local}><Icon name={f.local ? "laptop" : "wifi"} size={13} stroke={2} />{f.text}</div>
  {/if}
{/snippet}

<style>
  .acc {
    display: grid;
    padding: 4px;
    margin-bottom: 8px;
  }
  .ar {
    border-radius: calc(var(--r-lg) - 4px);
  }
  .ar + .ar {
    box-shadow: 0 -1px 0 var(--rim);
  }
  .ar.open + .ar,
  .ar + .ar.open {
    box-shadow: none;
  }
  .ar.open {
    background: var(--ink-2);
    box-shadow: 0 0 0 1px var(--rim-2);
    margin: 2px 0;
  }
  .ar-h {
    width: 100%;
    display: grid;
    grid-template-columns: 32px minmax(0, 1fr) auto 16px;
    gap: 12px;
    align-items: center;
    padding: 10px 12px;
    border: 0;
    border-radius: calc(var(--r-lg) - 4px);
    background: none;
    color: var(--pearl);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background 160ms;
  }
  .ar-h:hover {
    background: var(--hover);
  }
  .ar.open .ar-h:hover {
    background: none;
  }
  .ic {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: var(--ink-3);
    color: var(--haze);
  }
  .ar.open .ic {
    background: color-mix(in srgb, var(--think) 16%, transparent);
    color: var(--think);
  }
  .w {
    display: grid;
    gap: 1px;
    min-width: 0;
  }
  .w b {
    display: flex;
    align-items: center;
    gap: 7px;
    font: 600 var(--fs-body)/1.3 var(--sans);
  }
  .w > span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .w > span.warn {
    color: var(--wait);
  }
  .right {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .who {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    max-width: 230px;
    padding: 4px 10px 4px 5px;
    border: 1px solid var(--rim);
    border-radius: 999px;
    background: var(--ink-2);
    font-size: var(--fs-meta);
    white-space: nowrap;
  }
  .who-t {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ar.open .who {
    background: var(--ink);
  }
  .who.linked {
    padding-left: 10px;
    border-style: dashed;
    background: none !important;
    color: var(--haze);
  }
  .who.todo {
    padding-left: 10px;
    border-color: color-mix(in srgb, var(--think) 45%, transparent);
    background: color-mix(in srgb, var(--think) 14%, transparent) !important;
    color: var(--think);
    font-weight: 600;
  }
  .loc {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border-radius: 50%;
    color: var(--think);
    background: color-mix(in srgb, var(--think) 12%, transparent);
  }
  .loc.mac {
    color: var(--done);
    background: color-mix(in srgb, var(--done) 12%, transparent);
  }
  .chev {
    display: grid;
    place-items: center;
    color: var(--dusk);
    transition: transform 200ms;
  }
  .ar.open .chev {
    transform: rotate(90deg);
  }
  .logo {
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    border-radius: 5px;
    color: #fff;
    font: 700 10px/1 var(--sans);
  }
  .logo.same {
    background: none;
    color: var(--haze);
    box-shadow: inset 0 0 0 1px var(--dusk);
  }

  /* An open row. */
  .ar-b {
    display: grid;
    gap: 12px;
    padding: 2px 16px 16px 56px;
    animation: open 240ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes open {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  .lbl {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    color: var(--dusk);
    font: 600 var(--fs-small)/1.2 var(--sans);
    letter-spacing: 0.02em;
  }
  .lbl .faint {
    font-weight: 500;
    letter-spacing: 0;
  }
  .svcs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .svc {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 6px 11px 6px 6px;
    border: 1px solid var(--rim-2);
    border-radius: var(--r-md);
    background: var(--ink);
    color: var(--pearl);
    font: inherit;
    font-size: var(--fs-meta);
    cursor: pointer;
  }
  .svc:hover {
    border-color: var(--pearl-mid);
  }
  .svc.on {
    border-color: var(--think);
    box-shadow: inset 0 0 0 1px var(--think);
    background: color-mix(in srgb, var(--think) 8%, var(--ink));
  }
  .svc .k {
    display: inline-grid;
    color: var(--dusk);
  }
  .svc-note {
    margin-top: -4px;
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .models,
  .list {
    display: grid;
    gap: 2px;
    padding: 4px;
    border: 1px solid var(--rim);
    border-radius: 12px;
    background: var(--ink);
  }
  .model {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr);
    gap: 4px 10px;
    align-items: center;
    width: 100%;
    padding: 9px 10px;
    border: 0;
    border-radius: 9px;
    background: none;
    color: var(--pearl);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .model:hover:not(:disabled) {
    background: var(--hover);
  }
  .model:disabled {
    cursor: default;
  }
  .n {
    display: grid;
    gap: 1px;
    min-width: 0;
  }
  .n b {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-meta);
    font-weight: 600;
  }
  .n > span {
    color: var(--haze);
    font-size: var(--fs-small);
  }
  .radio {
    width: 16px;
    height: 16px;
    border: 1.5px solid var(--dusk);
    border-radius: 50%;
  }
  .radio.on {
    border: 5px solid var(--think);
  }
  .badge {
    padding: 3px 6px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--think) 18%, transparent);
    color: var(--think);
    font: 600 10.5px/1 var(--sans);
  }
  .badge.size {
    background: var(--ink-3);
    color: var(--pearl-mid);
  }
  .prog {
    grid-column: 2;
    height: 4px;
    overflow: hidden;
    border-radius: 2px;
    background: var(--ink-3);
  }
  .prog i {
    display: block;
    height: 100%;
    background: var(--think);
    transition: width 200ms;
  }
  .more {
    padding: 6px 10px 4px 38px;
    border: 0;
    background: none;
    color: var(--think);
    font: inherit;
    font-size: var(--fs-meta);
    text-align: left;
    cursor: pointer;
  }
  .row-btns {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .row-btns .more {
    padding-left: 10px;
  }
  .row-btns .more:first-child {
    padding-left: 38px;
  }
  .all {
    padding: 6px 10px 4px 38px;
  }
  .keybox,
  .callout {
    display: grid;
    gap: 10px;
    padding: 12px;
    border: 1px solid color-mix(in srgb, var(--wait) 35%, transparent);
    border-radius: 12px;
    background: color-mix(in srgb, var(--wait) 7%, var(--ink));
    font-size: var(--fs-meta);
  }
  .callout.think {
    border-color: color-mix(in srgb, var(--think) 35%, transparent);
    background: color-mix(in srgb, var(--think) 7%, var(--ink));
  }
  .line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px 12px;
  }
  .line.col {
    display: grid;
  }
  .line .t {
    display: grid;
    gap: 1px;
    min-width: 0;
    flex: 1 1 220px;
  }
  .line .t b {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: var(--fs-meta);
    font-weight: 600;
  }
  .line .t span {
    color: var(--haze);
    font-size: var(--fs-small);
  }
  .how {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  select {
    max-width: 300px;
  }
  .lock {
    display: grid;
    justify-items: start;
    gap: 8px;
    padding: 12px;
    border: 1px solid var(--rim);
    border-radius: 12px;
    background: var(--ink);
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .lock :global(.icon) {
    vertical-align: -2px;
  }
  .keyline {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto;
    gap: 10px;
    align-items: center;
    padding: 8px 10px;
    font-size: var(--fs-meta);
  }
  .keyfield {
    padding: 4px 10px 12px 38px;
  }
  .adv {
    display: grid;
    gap: 10px;
    padding-top: 12px;
    border-top: 1px dashed var(--rim-2);
  }
  .onoff {
    padding-bottom: 12px;
    border-bottom: 1px solid var(--rim);
  }
  .opts {
    display: grid;
    gap: 14px;
    padding-top: 12px;
    border-top: 1px solid var(--rim);
  }
  .off {
    opacity: 0.5;
  }
  .kinds {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 8px;
  }
  .kind {
    display: grid;
    align-content: start;
    gap: 8px;
    padding: 12px;
    text-align: left;
    color: inherit;
    font: inherit;
    font-size: var(--fs-meta);
    background: var(--ink);
    border: 1px solid var(--rim);
    border-radius: 12px;
    cursor: pointer;
  }
  .kind:hover {
    background: var(--hover);
  }
  .kind.on {
    border-color: var(--think);
    box-shadow: inset 0 0 0 1px var(--think);
  }
  .sample {
    padding: 8px 10px;
    border-radius: var(--r-sm);
    background: var(--hover);
    font-size: var(--fs-small);
    line-height: 1.45;
  }
  .sample.timed {
    display: grid;
    grid-template-columns: auto auto 1fr;
    gap: 2px 8px;
  }
  .sw {
    font-weight: 600;
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    color: var(--think);
    font-size: var(--fs-meta);
  }
  .foot.mac {
    color: var(--done);
  }
  .small {
    font-size: var(--fs-meta);
  }
  .note {
    margin-top: 12px;
  }

  @media (max-width: 620px) {
    .ar-h {
      grid-template-columns: 32px minmax(0, 1fr) 16px;
    }
    .right {
      grid-column: 2;
      grid-row: 2;
      justify-self: start;
    }
    .chev {
      grid-row: 1;
      grid-column: 3;
    }
    .ar-b {
      padding-left: 14px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ar-b {
      animation: none;
    }
    .chev {
      transition: none;
    }
  }
</style>
