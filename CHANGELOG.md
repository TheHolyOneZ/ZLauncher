# Changelog

Every ZLauncher release, newest first. The first paragraph of each version becomes the update notice inside the launcher; the whole section becomes the release notes.

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

[0.2.0]: https://github.com/TheHolyOneZ/ZLauncher/releases/tag/v0.2.0
[0.1.0]: https://github.com/TheHolyOneZ/ZLauncher/releases/tag/v0.1.0
