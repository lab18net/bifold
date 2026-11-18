# Naming: five candidates

Constraint set: one word (or one coined compound), pronounceable, spellable
after hearing it once, no direct clash with an existing file manager, no
negative or medical first meaning, works as a CLI binary and as a macOS app
name, and still sounds like a Lab18 product (GateKeeper, SploitHound, Synapse,
Amethyst — evocative, slightly industrial, not cute).

Collision checks were ordinary web searches (product name + "file manager" /
"app" / "software"), not trademark clearance. Treat the collision column as
"what a search turns up", and do a proper register search before spending money
on a domain or a logo.

| # | Name | Metaphor | Binary | What a search turns up |
|---|---|---|---|---|
| 1 | **Bifold** | A bifold door: two leaves, one frame, opens by folding | `bifold` | OpenWallet Foundation's `bifold-wallet` (React Native identity wallet, Apache-2.0) and a few small tools. No file manager, no strong consumer brand. |
| 2 | **Diptych** | A two-panelled painting; literally "folded twice" | `dip` | A Canvas LMS grading companion (Chrome extension) and a small macOS Xcode app. Semantically perfect, mildly crowded namespace, awkward to spell for some. |
| 3 | **Shunt** | Railway shunting: moving cars between tracks | `shunt` | Almost nothing in this category — small hobby projects. Most distinctive of the five; also the most "engineer's choice", and the medical/electrical senses of the word are a mild tax. |
| 4 | **Dosido** | Do-si-do: two dancers circling, passing and returning | `dosido` | Effectively clear. Memorable, good logo geometry (two arcs), but reads as playful rather than a tool. |
| 5 | **Windlass** | A winch that hauls a load between two points | `wl` / `windlass` | A self-hosted Docker Compose panel (`windlass.run`). Different category, same "operator tool" feel. Longest of the five to type. |

Rejected during the search, for the record: **Mullion** — a C++17 dual-pane TUI
named `mullion-fileman` already exists, i.e. a direct competitor; **Tandem**,
**Janus**, **Sash** — heavily used in dev tooling and window management;
**Haul** — an existing macOS download manager with a `haul` command.

## Recommendation

**Bifold** — it says what the product is without a tagline (two panes, one
frame), it is easy to type and to say, `bifold` reads well as a shell command,
and `Bifold.app` reads well in a Dock. The one real collision is an unrelated
identity wallet, so a domain and a trademark search are worth doing before
launch. Runner-up: **Shunt**, if the name should stand out over being obvious.

The repo is scaffolded with this name. Everything name-dependent is a string
replacement across tracked files:

```bash
scripts/rename-product.sh Shunt --dry-run    # show what would change
scripts/rename-product.sh Shunt              # rewrite, then:
cargo build --workspace && cargo test --workspace
```

That rewrites `Bifold` (display), `bifold` (binaries, config directory, crate
prefix, env vars) and `BIFOLD` (env var prefixes) consistently, including the
key table in the README and the built-in help text.
