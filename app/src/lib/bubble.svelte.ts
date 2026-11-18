// Yoru at the bottom right of the pages (YoruBubble.svelte): whether his
// chat is open, and words to start it with, so a page (the Memory's "tell
// Yoru") can open it already pointed at something.

class Bubble {
  open = $state(false);
  /** Put in the field when it opens, for the user to finish and send. */
  draft = $state("");

  show(draft = "") {
    this.draft = draft;
    this.open = true;
  }

  hide() {
    this.open = false;
  }
}

export const bubble = new Bubble();
