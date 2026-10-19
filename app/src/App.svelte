<script lang="ts">
  import { i18n } from "./lib/i18n/index.svelte";
  import Board from "./lib/pages/Board.svelte";
  import Connectors from "./lib/pages/Connectors.svelte";
  import Conversation from "./lib/pages/Conversation.svelte";
  import Inbox from "./lib/pages/Inbox.svelte";
  import Memory from "./lib/pages/Memory.svelte";
  import Routines from "./lib/pages/Routines.svelte";
  import Settings from "./lib/pages/Settings.svelte";
  import Today from "./lib/pages/Today.svelte";
  import Setup from "./lib/Setup.svelte";
  import Sidebar from "./lib/Sidebar.svelte";
  import { store } from "./lib/store.svelte";
  import WindowBar from "./lib/WindowBar.svelte";

  store.start();
  i18n.start();

  const PAGES = { today: Today, conversation: Conversation, inbox: Inbox, connectors: Connectors, routines: Routines, memory: Memory, yoru: Board, settings: Settings };
  const Page = $derived(PAGES[store.page]);
  let main: HTMLElement | undefined = $state();
  // Each page starts at its top.
  $effect(() => {
    void store.page;
    main?.scrollTo({ top: 0 });
  });
</script>

<div class="app">
  <Sidebar />
  <main bind:this={main}>
    <Page />
  </main>
  <WindowBar />
  {#if store.onboarding && !store.onboarding.done}<Setup />{/if}
</div>

<style>
  .app {
    position: relative;
    display: grid;
    grid-template-columns: 228px minmax(0, 1fr);
    height: 100vh;
  }
  main {
    position: relative;
    overflow-y: auto;
    padding-top: 18px;
  }
</style>
