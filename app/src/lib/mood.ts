// Yoru's face for what's said in the chat: a reaction from the board's set
// (YoruBoard plays them) picked from the words of a line, in English or
// Italian. Plain word lists: it only has to look right, not understand.

import type { ChatLine } from "./types";

type Rule = { name: string; words: RegExp };

const word = (list: string) => new RegExp(`(^|[^\\p{L}])(${list})([^\\p{L}]|$)`, "iu");

/** What the user says to him. */
const YOURS: Rule[] = [
  { name: "love", words: word("grazie|thanks|thank you|ti voglio bene|love you") },
  { name: "proud", words: word("bravo|bravissimo|perfetto|ottimo|great job|well done|perfect|nice") },
  { name: "wave", words: word("ciao|hello|hi|hey|buongiorno|buonasera|good morning|good evening") },
  { name: "giggle", words: word("ahah|ahahah|haha|lol|😂") },
];

/** What he answers. */
const HIS: Rule[] = [
  { name: "sad", words: word("purtroppo|unfortunately|non riesco|non posso|i can't|i cannot|couldn't|non sono riuscito") },
  { name: "shy", words: word("scusa|scusami|mi dispiace|sorry|apologies") },
  { name: "wave", words: word("ciao|hello|hi|hey|buongiorno|buonasera|good morning|good evening|a presto|see you") },
  { name: "surprised", words: word("wow|incredibile|accidenti|amazing|whoa") },
  { name: "proud", words: word("fatto|completato|ecco fatto|done|all set|finished|✅") },
  { name: "excited", words: word("ottimo|fantastico|benissimo|great|awesome|excellent|🎉") },
];

/** The reaction a new line calls for, if any. */
export function reactionFor(line: ChatLine): string | null {
  const text = line.text.trim();
  if (!text) return null;
  if (line.from === "yoru") {
    if (text.startsWith("(error:")) return "sad";
    const hit = HIS.find((r) => r.words.test(text));
    if (hit) return hit.name;
    if (text.endsWith("?")) return "curious";
    // Something done with his tools: pleased with himself.
    if (line.actions?.some((a) => a.ok)) return "happy";
    return null;
  }
  return YOURS.find((r) => r.words.test(text))?.name ?? null;
}
