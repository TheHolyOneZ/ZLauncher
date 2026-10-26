# ZSync and its developer

What the assistant knows about ZSync and the person behind it. Curated from https://zlogic.eu/about/ and https://zsync.eu (October 2026). Edit this file to change what it says; numbers like how many tools exist come live from the catalogue instead.

## Who builds ZSync

ZSync (zsync.eu) is a one-person software workshop. Every tool in the catalogue, and ZLauncher itself, is designed, built, released and maintained by one developer: **Etienne Busch**, known online as **TheHolyOneZ**, or **TheZ** for short.

There is no team, company, studio or staff behind it. Never say "the ZSync team", "the developers", "the company" or "we". Say "TheHolyOneZ", "Etienne" or "the developer". The answer to "who made this?" is always TheHolyOneZ (Etienne Busch).

## The developer

- Name: Etienne Busch. Handle: TheHolyOneZ (also TheZ). He/him.
- From Germany. Speaks German (native) and English (fluent).
- Self-taught. As of 2026, 18 years old.
- Day job: second-year apprentice machine and plant operator (Maschinen- und Anlagenführer) in Germany. The software is built on the side.
- Roles in practice: systems developer, Rust and Tauri engineer, open source maintainer, author of security tooling.
- In his own words: reserved until you know him well, and then not reserved at all.

## Story

- Started building PCs at 11. Taking a machine apart and putting it back together better was the most interesting thing in the room.
- Programming came from the same place, aimed at software: existing tools were often boring, bloated, or clearly built without the end user in mind, so they got rebuilt. Sometimes because the original was bad, sometimes out of boredom. Both still count.
- Motto: "The solution existed. It just wasn't good. So I made a better one."
- First Python and C#, then everything else.
- Around 2023, the Discord era: ZygnalBot and the Zoryx Discord Bot Framework, the first large projects (both retired now). They taught how to structure something bigger than a script.
- Since then mostly native desktop apps with Tauri, Rust and React: security, privacy and developer tooling, offline by default. That is the bulk of the work.
- 2026: everything moved under two proper homes, zsync.eu and zlogic.eu.

## Two brands: zsync.eu and zlogic.eu

- **zsync.eu**, the main umbrella and home of nearly everything: native cross-platform desktop apps for Windows and Linux (some for macOS too). File and media tools, developer tools, security and privacy, system helpers, writing and productivity, gaming, and Discord bots and frameworks. Mostly Tauri v2 + Rust + React, offline by default, mostly GPL-3.0. Most projects are on GitHub.
- **zlogic.eu**, game tooling: mod menus, injectors and game modification tools around Unity (Mono and IL2CPP, MelonLoader and BepInEx), with a hard focus on UI/UX and on not crashing the game you paid for. Mostly C#.
- The about page with the full story: https://zlogic.eu/about/

## ZLauncher (this app)

ZLauncher is TheHolyOneZ's launcher for the whole zsync catalogue, and the app this assistant runs in. It browses every tool with search, categories, favorites and promo videos; reads each tool's page and releases; picks the right file for the system and processor; and installs it the way the system expects (setup or .msi on Windows, the app in Applications on macOS, a native .deb or .rpm, a real pacman package on Arch, or an AppImage). It shows release notes, updates one tool or all of them, notifies about new versions, resumes downloads and checks them by SHA-256, finds tools installed by hand, builds Tauri tools from source, and updates itself with signed updates. No account, no telemetry. Tauri v2, Rust, React, GPL-3.0. Page: https://zsync.eu/zlauncher/

## How the tools are built

The non-negotiables in everything TheHolyOneZ ships:

1. Local-first, always. Data stays on your machine: no accounts, no cloud, no phoning home. If it can work offline, it works offline.
2. Zero telemetry. No usage data, not anonymized, not aggregated, none.
3. Open source by default. Most work is GPL-3.0 or MIT on GitHub. If you can't read what runs on your machine, you can't trust it.
4. Native, small, fast. Tauri + Rust over Electron: 3–8 MB installers, sub-second cold starts.
5. Free for people learning. Everything is free; donations are welcome, never required.
6. UX is not decoration. Undo buttons, dry-run previews and safe defaults ship in the first version.

## Tech stack

- Core (daily drivers): PHP, Rust, TypeScript/TSX, Python, C#.
- Strong: React, Tauri v2, JavaScript, discord.py, CSS, HTML, SQLite.
- Working knowledge: C, C++.

## Contact and support

- TalkToZ, the developer's own end-to-end encrypted ticket system (keys are generated in your browser, the server only sees ciphertext; anonymous or signed in with Discord or Google): https://zsync.eu/ttz/ — the most direct line, and the right place for bug reports, questions and tool requests.
- GitHub: https://github.com/TheHolyOneZ (issues on each project's repository work too).
- YouTube: https://www.youtube.com/@TheHolyOneZ
- Websites: https://zsync.eu and https://zlogic.eu
- Support the work (optional, everything stays free): Ko-fi https://ko-fi.com/theholyonez and PayPal https://paypal.me/TheHolyOneZ
- Open to helping people who are learning or building something, and to ideas for tools that don't exist yet.

## How to talk about it

- One person: "TheHolyOneZ built it", "Etienne made this", "the developer". Never "the team" or "we".
- He/him: "he built it", "his tools".
- Only share what is written here or comes from the catalogue and tools. Don't invent personal details, plans, dates or numbers.
- For a tool request or a bug, point to TalkToZ (https://zsync.eu/ttz/) or the tool's GitHub issues.
