# About this fork

This fork is called **Xpedited**. It is a fork of Xodus, not a replacement for
it and not a competitor to it.

**The project this comes from is [Xodus](https://github.com/xodus-gaming/xodus).
Go there first.** If you found this fork looking for a way to run Store and
Game Pass titles on Linux, Xodus is the project doing that work, and it is the
one worth your stars, your issues and your help. Their Discord is linked from
their README.

Credit for essentially all of this belongs upstream. The hard parts - working
out the MSIXVC container format, the device and user authentication, the
licensing and content keys, the on-demand executable decryption, the whole
shape of the thing - are theirs. What this fork adds sits on top of that
foundation and would be worth nothing without it.

This is a modified copy under GPL-3.0-or-later, the same licence Xodus uses.
Upstream holds the copyright on everything it wrote. The changes below were
made from 2026-09-18 onwards.

This fork is not endorsed by, affiliated with, or a competitor to the Xodus
project. It exists because of one person's games and one person's machine.
The name is only there so the two can be told apart in a launcher list and a
process table; where the code says `xodus`, that is upstream's and it keeps
upstream's name.

## Why there is an LLM in the commit history

These changes were written with an AI assistant. That deserves an explanation
rather than a footnote, because upstream's contribution policy
(<https://github.com/xodus-gaming/.github/blob/main/CONTRIBUTING.md>) says:

> Reverse engineering efforts driven by LLM are not allowed
> LLM assisted code will be rejected from most repositories

Xodus follows Wine's clean room guidelines. Clean room provenance is what lets
a reimplementation survive contact with lawyers, and it only works if nobody
in the room has read the wrong thing. A single contributor pasting in
machine-generated reverse engineering can put years of other people's careful
work at risk.

## Please do not send any of this upstream

Just don't please I did use AI too make this because I wanted to see what was possible and give back something somewhat useful.

## What this fork changes

- `msixvc`: the user data region's start is a page index, and was being seeked
  to as a byte offset. Packages whose header type happened to be non-zero were
  skipped silently; two titles could not be downloaded at all.
- `xodus-cli run`: hands the running title its package identity, resolves the
  signed in user, and defaults .NET to invariant globalization, without which
  no managed title starts under Wine.
- `xodus-cli heroic`: exports an extracted game to the Heroic Games Launcher.
- `xodus-cli library` / `metadata`: list the account's titles and their store art.
- `xodus-service`: handles SIGTERM as well as SIGINT and reclaims a stale socket.

The GDK runtime changes live in a separate tree, as patches against Wine and
against upstream's `xgameruntime` submodule. The same provenance applies to
them, and so does everything above.

## Also worth looking at

[Ferestre](https://github.com/icex/ferestre)
