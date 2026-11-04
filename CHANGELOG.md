# Changelog

Every ZLauncher release, newest first. The first paragraph of each version becomes the update notice inside the launcher; the whole section becomes the release notes.

## [0.3.0] - 2026-11-04

Talk to TheZ, inside the launcher: start an end-to-end encrypted chat with the developer, anonymously or with your TalkToZ account, send files, and get notified when he replies. Downloads that stall or crawl now continue from the tool's GitHub release.

### New: Talk to TheZ

- **A direct line.** Help, requests, feedback, collabs or just hi, straight to TheHolyOneZ through TalkToZ (zsync.eu/ttz). Encrypted on your computer with the same NaCl boxes as the website, so only he can read it.
- **Anonymous that stays reliable.** An anonymous chat needs no account, and nothing in a browser can lose it: its key and access token live in the launcher. Every anonymous chat has keys of its own, so two of them can't be linked.
- **A name if you want one.** Set a name for anonymous chats (Settings or the new-chat form); TheHolyOneZ sees it instead of "Anonymous". It is encrypted too.
- **Your account, signed in through the browser.** Sign in with Discord or Google on the website and confirm; the launcher is connected. Chats of your account show up, and new ones can be linked to it. Signing out revokes the connection.
- **Replies come to you.** While a chat is open, the launcher checks every minute (every few minutes from the tray). With the launcher in front you get a notice inside it; otherwise a system notification that never shows the subject or the message (Settings → Talk to TheZ).
- **Send files.** Pictures, videos, zips, PDFs and text or code files up to 100 MB, five per message: pick them or drop them on the window. Each file is encrypted on your computer with its own key, so the server only stores bytes it can't read. Folders go in a zip; programs and installers are refused.
- **Files stay inert.** Nothing received is ever opened, unpacked or run. Every file is judged again on arrival, and its real content has to match its type. Pictures are decoded in memory-safe Rust and shown as a fresh copy, code is shown as plain text, zips and videos can only be saved. Saved files are marked as downloaded, so the system warns before anything in them runs.
- **Photos leave their location behind.** Location and device data is removed from JPEG and PNG photos before they are sent. Formats that can keep it (videos, for example) say so before you send.
- **Full formatting.** Bold, italic, underline, strikethrough, spoilers, headings, lists, checklists, quotes, tables, links and code blocks with a copy button, for your own messages too, exactly as on the website. A formatting bar, a preview and the same shortcuts are in the composer.
- **Edit and delete your messages.** Changes reach the other side by themselves. A deleted message is really gone from the server, with its files.
- **The assistant can help with your chats, if you let it.** Turn on "Let the assistant use my chats" in Settings and it can read and search them, and draft a new chat, a reply, an edit or a delete. Everything it writes waits on a card you can change, and nothing goes out until you press the button. What it reads goes to your AI provider, so this is off by default. It never sees file contents.

### Changed

- Downloads no longer hang: no data for 20 seconds resumes the download, and a download crawling below about 128 KB/s continues from the same byte on the tool's GitHub release. The SHA-256 check at the end covers the whole file. Reported in [#1](https://github.com/TheHolyOneZ/ZLauncher/issues/1).
- The Downloads page says where a download continues from when it switches.

## [0.2.0] - 2026-10-26

ZLauncher now has an AI assistant: describe what you need and it finds the right tool, explains it, reads its README and installs, downloads, builds or updates it for you. It works with a local model through Ollama, or with your own OpenAI, Anthropic, Gemini or OpenAI-compatible key.

### New: the assistant

- **Ask in your own words.** "Something to test how fast I type", "are there updates for my tools?", "encrypt files before uploading". It searches the whole catalogue, typos and all, and answers with cards for the tools it means.
- **Does the work.** Install, download (to `Downloads/ZLauncher`), build from source, update one tool or all of them, open a tool, or uninstall it after you confirm. It asks for the Linux admin unlock only when a step needs it.
- **Reads the source.** It can open a tool's GitHub README (outline first, then the section it needs), search it with regular expressions, and look up the repository: stars, license, recent releases, open issues.
- **Knows your system.** It sees your OS, processor and Linux install format, what you have installed and what has updates, and points you to what runs on your computer.
- **Knows ZSync.** Ask who makes all this, how to report a bug or how to support the work; it knows the developer and the links.
- **Real thinking.** With models that reason (Claude, Gemini, Qwen 3, DeepSeek R1 and others) you can watch it think, live. Every tool call is shown as a step you can open. Thinking can be turned off in Settings.
- **Streams as it writes.** Answers appear word by word, a reply can be stopped at any time, and it keeps going in the background if you close the panel.
- **Chats are kept.** Earlier chats stay in a list, get a title automatically, and can be deleted (with undo). `Ctrl N` (`⌘ N` on macOS) starts a new one.
- **Your choice of model.** Ollama (local, the default: no account, and your chats never leave your computer), OpenAI, Anthropic, Gemini, or any OpenAI-compatible server such as LM Studio or llama.cpp. Settings has "Test connection" and lists the models your provider offers. API keys stay on your computer.

### Changed

- Dropdowns scroll when they have many options, and long option names are cut off cleanly instead of stretching the menu.
- The window can be resized from its edges even while a dialog is open.

### Under the hood

- Each provider's streaming format (Anthropic, OpenAI-compatible, Gemini, Ollama) is tested against a local mock server that speaks that API, including thinking, tool calls split across chunks, errors and retries.
- Local model servers are reached without the system proxy, so a proxy on Windows or macOS can't block Ollama.

## [0.1.0] - 2026-10-20

The first release.

- Browse every tool on zsync.eu: search, categories, favorites and promo videos.
- Finds each tool's installers on its website, release manifests, checksum lists and GitHub releases, and picks the right one for your system and processor.
- Installs the way your system expects: silent setup or `.msi` on Windows, the app in Applications on macOS, `.deb` or `.rpm` through the package manager, a real pacman package on Arch, or an AppImage.
- Updates with release notes, one tool or all at once, with notifications.
- Resumable downloads, checked against SHA-256 before anything is installed.
- Finds tools you installed before by hand.
- Builds Tauri tools from their GitHub source, with a check of what your system needs first.
- Updates itself with signed updates.

[0.3.0]: https://github.com/TheHolyOneZ/ZLauncher/releases/tag/v0.3.0
[0.2.0]: https://github.com/TheHolyOneZ/ZLauncher/releases/tag/v0.2.0
[0.1.0]: https://github.com/TheHolyOneZ/ZLauncher/releases/tag/v0.1.0
