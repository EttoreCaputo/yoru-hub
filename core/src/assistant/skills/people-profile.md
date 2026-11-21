---
name: people-profile
description: How much to keep about a person, and how: a stub at the first real mention, more as they come back, a full profile once they matter. Use it whenever you create or grow a note in people/.
---
# Notes about people

One note per person, in `people/<first-last>.md` (lowercase, dashes), even when they are called in different ways: the other names go in `aliases`. Look for the note (the index, brain_search) before creating one, so nobody gets two.

## How much

- **Mentioned in passing** (a name in a list, someone the user won't meet again): no note.
- **First real mention** (who they are to the user, what they do): a stub. The frontmatter (`type: person`, a `description` of one line that says who they are, `aliases`), and a few lines.
- **Back three times or more**, in conversations or meetings: grow it. What they do and where, how the user knows them, what they are working on with the user, what they care about.
- **After a meeting with them, or about eight mentions**: a full profile, with the sections below.

## The sections

```
# Name

## Now
- Role, company, how the user knows them [Source: ...]
- What they are working on with the user [Source: ...]

## What they care about
## How to work with them
## Open threads
- What one owes the other, with the date

## Timeline
- YYYY-MM-DD what happened [Source: ...]
```

Keep only what the user would want to remember about them. Every fact says where it comes from. What is no longer true moves from Now to the timeline, with the date it stopped. The `description` line is what the index shows: keep it current, since it is how you find the note later. Health, money and family matters go in a separate note with `private: true`, never in this one.
