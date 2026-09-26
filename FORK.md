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
work at risk. That policy is entirely reasonable and this fork is not an
argument with it.

The honest account of how this came about: this started as one person wanting
to play games he had paid for on the operating system he uses, working with an
AI assistant because that is how he works. The contribution policy lives in the
organisation's `.github` repository rather than in this one, and neither of us
went looking for it until the work was already done. Had we read it first,
this would have been a fork from the beginning instead of becoming one.

So it is a fork, and it stays downstream.

## Please do not send any of this upstream

Not as a pull request, not as a patch in their Discord, not as an issue with a
diff in it. Reading LLM-derived reverse engineering is exactly the thing their
clean room policy exists to prevent, and doing it to them by accident would be
a genuinely rotten way to repay the project this is built on.

Two things are still fine, and upstream welcomes both: bug reports that
describe only a symptom and how to reproduce it, and documentation.

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

[Ferestre](https://github.com/icex/ferestre) is another downstream project with
the same goal, further along in several places. Its patch series answered two
questions this fork had got wrong by guesswork.
