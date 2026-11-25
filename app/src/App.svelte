<script lang="ts">
  import { i18n } from "./lib/i18n/index.svelte";
  import Animations from "./lib/pages/Animations.svelte";
  import Connectors from "./lib/pages/Connectors.svelte";
  import Chat from "./lib/pages/Chat.svelte";
  import Home from "./lib/pages/Home.svelte";
  import MeetingView from "./lib/pages/MeetingView.svelte";
  import Inbox from "./lib/pages/Inbox.svelte";
  import Meetings from "./lib/pages/Meetings.svelte";
  import Todos from "./lib/pages/Todos.svelte";
  import Memory from "./lib/pages/Memory.svelte";
  import Tasks from "./lib/pages/Tasks.svelte";
  import Settings from "./lib/pages/Settings.svelte";
  import Spoken from "./lib/pages/Spoken.svelte";
  import PermissionGate from "./lib/PermissionGate.svelte";
  import Setup from "./lib/Setup.svelte";
  import Sidebar from "./lib/Sidebar.svelte";
  import { store } from "./lib/store.svelte";
  import { tabs } from "./lib/tabs.svelte";
  import WindowBar from "./lib/WindowBar.svelte";
  import YoruBubble from "./lib/YoruBubble.svelte";
  import { SETTINGS_PAGES } from "./lib/icons/pages";
  import { look } from "./lib/look.svelte";
  import ActivityPanel from "./lib/ui/ActivityPanel.svelte";
  import MeetingPanel from "./lib/ui/MeetingPanel.svelte";
  import { page as meetingsPage } from "./lib/meetings-page.svelte";
  import PocketDialog from "./lib/ui/PocketDialog.svelte";
  import Icon from "./lib/ui/Icon.svelte";
  import { on } from "./lib/api";

  store.start();
  i18n.start();
  look.start();
  // The notch's Open on a summary that's ready.
  on<string>("open-meeting", (id) => tabs.open({ kind: "meeting", id }));

  const PAGES = { home: Home, inbox: Inbox, connectors: Connectors, tasks: Tasks, memory: Memory, meetings: Meetings, todos: Todos, spoken: Spoken, animations: Animations, settings: Settings };
  const Page = $derived(PAGES[store.page]);
  // Services, Spoken and Animations are opened from Settings, and lead back to it.
  const fromSettings = $derived(!tabs.shown && (SETTINGS_PAGES as readonly string[]).includes(store.page));
  // The Home, Memory and a conversation fill the card and scroll inside; the other pages scroll here.
  const fill = $derived(tabs.shown ? tabs.shown.kind === "chat" : store.page === "home" || store.page === "memory" || store.page === "meetings");
  // Yoru waits at the bottom right of the pages where he isn't already the page
  // (the Meetings page has the field to ask him at its bottom).
  const bubbled = $derived(tabs.shown ? tabs.shown.kind !== "chat" : store.page !== "home" && store.page !== "meetings");
  // On the Meetings page the right side is the meeting picked, or today's.
  const meetingSide = $derived(!tabs.shown && store.page === "meetings");
  let main: HTMLElement | undefined = $state();
  // Each page starts at its top.
  $effect(() => {
    void store.page;
    void tabs.shown;
    main?.scrollTo({ top: 0 });
  });
</script>

<!-- The frame: the bar on top, the sidebar on the left, the page as a card,
     and the activity panel on the right (on Meetings, the meeting's panel). -->
<div class="app" class:folded={!look.sidebar}>
  <WindowBar />
  <Sidebar />
  <main bind:this={main} class:fill>
    {#if fromSettings}
      <div class="back">
        <button class="btn quiet small" onclick={() => store.go("settings")}>
          <Icon name="chevron-left" size={16} />{i18n.t.settings.back}
        </button>
      </div>
    {/if}
    {#if tabs.shown?.kind === "chat"}
      {#key tabs.shown.id}<Chat id={tabs.shown.id} />{/key}
    {:else if tabs.shown?.kind === "meeting"}
      {#key tabs.shown.id}<MeetingView id={tabs.shown.id} />{/key}
    {:else}
      <Page />
    {/if}
  </main>
  {#if bubbled && !(store.onboarding && !store.onboarding.done)}<YoruBubble />{/if}
  {#if meetingSide}{#if look.activity || meetingsPage.picked}<MeetingPanel />{/if}{:else if look.activity}<ActivityPanel />{/if}
  {#if store.pocketShown && store.pocket}<PocketDialog pocket={store.pocket} onclose={() => (store.pocketShown = false)} />{/if}
  <!-- The first run asks for the permissions itself; after it, the gate
       waits for the microphone if it was taken away. -->
  {#if store.onboarding && !store.onboarding.done}<Setup />{:else if store.micMissing}<PermissionGate />{/if}
</div>

<style>
  .app {
    position: relative;
    /* The sidebar's width, which the window bar's tabs line up with too. */
    --sidebar-w: 236px;
    display: grid;
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr) auto;
    grid-template-rows: 48px minmax(0, 1fr);
    height: 100vh;
    transition: grid-template-columns 240ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .app.folded {
    grid-template-columns: 0 minmax(0, 1fr) auto;
  }
  main {
    grid-column: 2;
    grid-row: 2;
    position: relative;
    overflow-y: auto;
    margin: 0 10px 10px 0;
    padding-top: 8px;
    border-radius: 14px;
    background: var(--night);
    box-shadow: var(--card-shadow);
    transition: margin 240ms;
  }
  .folded main {
    margin-left: 10px;
  }
  main.fill {
    padding-top: 0;
    overflow: hidden;
  }
  /* Lined up with the page under it, which then starts a little higher. */
  .back {
    max-width: 880px;
    margin: 16px auto -26px;
    padding: 0 40px;
  }
  .back button {
    margin-left: -10px;
    gap: 4px;
  }
</style>
