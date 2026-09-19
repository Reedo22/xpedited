# About this fork

This is a modified copy of [Xodus](https://github.com/xodus-gaming/xodus), which
is licensed GPL-3.0-or-later. This fork keeps that licence. Upstream holds the
copyright on everything it wrote; the changes described below are additions on
top of it, made from 2026-09-18 onwards.

## Provenance, stated plainly

**The changes in this fork are LLM-assisted.** Upstream's contribution policy
(<https://github.com/xodus-gaming/.github/blob/main/CONTRIBUTING.md>) does not
accept LLM-assisted code, and follows Wine's clean room guidelines:

> Reverse engineering efforts driven by LLM are not allowed
> LLM assisted code will be rejected from most repositories

That policy is theirs to set and this fork exists rather than arguing with it.
Nothing here is offered to upstream, and it should not be sent to them: a
clean room stops being one the moment its contributors read work like this.
Bug reports that describe only a symptom, and documentation, remain fine to
share - upstream welcomes both.

This fork is not endorsed by or affiliated with the Xodus project.

## What changed

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
against upstream's `xgameruntime` submodule; the same provenance applies to
them.
