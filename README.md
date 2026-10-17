# OpenRouter Chat — reMarkable Paper Pro

Chat with any [OpenRouter](https://openrouter.ai) model directly on your
reMarkable Paper Pro. It runs as a standalone external process under
[XOVI](https://github.com/asivery/xovi) +
[AppLoad](https://github.com/asivery/rm-appload) — no xochitl injection, no
boot-bank risk.

- **Pen whiteboard input** — write your prompt with the marker on a low-latency
  ink pad, then send it as transcribed text (*Send Ink-to-Text*) or as a raw
  image (*Send Image*, an inline `image_url` payload).
- **On-screen keyboard** — toggle a touch QWERTY keyboard and type a normal
  text prompt. Includes a SHIFT key (tap for one capital letter) and
  multi-tap punctuation keys: `, . ?` cycles `,` → `.` → `?` → `!`, and
  `- / :` cycles `-` → `/` → `:`.
- **Free-model picker** — the app lists only free OpenRouter models (`:free`
  variants and zero-price models). Tap the header model name for a dropdown
  list, or use the ◀ ▶ arrows to step through them.
- **Settings menu (⚙)** — tap the gear in the header to see your masked API
  key, run **Test API** (reports `API OK — N free models` or the full error),
  change the model, adjust text size (A− / A+), and cycle the text colour
  (Ink black / Navy / Sepia).
- **Chat bubbles for everything** — your message appears instantly when you
  press Send/⏎, a pending bubble shows while the network works, and the
  answer *or the full error text* lands in its own bubble. No more truncated
  header errors: failures are readable in the transcript and in the device
  log.
- **Markdown answers** — assistant replies render as real Markdown (headings,
  lists, code blocks, tables) through the on-device blitz/stylo HTML engine.
- **Scrollable transcript** — tap the upper half of the chat area to scroll up,
  the lower half to scroll down.

---

## Install

You need [xovi](https://github.com/asivery/xovi) and
[rm-appload](https://github.com/asivery/rm-appload) installed on the device
first (one-time, free). Then either download a release or build it yourself.

### Option A — download a release (recommended)

1. Grab `openrouter-chat-appload.tar.gz` from the
   [Releases](../../releases) page (or the *Actions* tab → latest run →
   *openrouter-chat-appload* artifact).
2. Copy it to the device and unpack it into the AppLoad directory:

   ```sh
   scp openrouter-chat-appload.tar.gz root@remarkable:/tmp/
   ssh root@remarkable
   mkdir -p /home/root/xovi/exthome/appload/openrouter-chat
   tar -xzf /tmp/openrouter-chat-appload.tar.gz \
       -C /home/root/xovi/exthome/appload/openrouter-chat --strip-components=1
   chmod +x /home/root/xovi/exthome/appload/openrouter-chat/openrouter-chat
   ```

### Option B — build it yourself

Host prerequisites: [rustup](https://rustup.rs), Zig 0.13, and
`cargo-zigbuild`:

```sh
rustup target add aarch64-unknown-linux-musl
cargo install cargo-zigbuild --locked
./tools/build-appload.sh
```

The script cross-compiles the static musl binary and assembles everything into
`dist/openrouter-chat/`, packed as `dist/openrouter-chat-appload.tar.gz`.
Copy that directory to the device exactly as in Option A, step 2.

Either way, the final device layout is:

```
/home/root/xovi/exthome/appload/openrouter-chat/
├── openrouter-chat            # the app (static aarch64 musl binary)
├── external.manifest.json     # AppLoad manifest (name/application/qtfb)
├── icon.png                   # launcher icon
└── INSTALL.txt                # these steps, on the device
```

---

## Add your OpenRouter API key

Create a key at <https://openrouter.ai/keys>, then edit the manifest **on the
device**:

```sh
ssh root@remarkable
vi /home/root/xovi/exthome/appload/openrouter-chat/external.manifest.json
```

Add an `environment` block so AppLoad passes the key to the process:

```json
{
  "name": "OpenRouter Chat",
  "application": "openrouter-chat",
  "qtfb": true,
  "environment": {
    "OPENROUTER_API_KEY": "sk-or-v1-your-key-here"
  }
}
```

Save, then launch **OpenRouter Chat** from the XOVI → AppLoad menu. The header
shows how many models were loaded once the list arrives.

Notes:

- The key is read from the `OPENROUTER_API_KEY` environment variable at
  startup — nothing is fetched or sent until you tap a Send button.
- The key is stored as plain text on the device (anyone with root/SSH access
  can read the manifest). Don't reuse a high-limit production key.
- No key set? The app still opens; the header reads "No API key" and
  Settings shows the key as `not set`. Set the key and relaunch.
- Key set but requests fail? The full error (HTTP status, API message, or
  TLS/network detail) appears in an error bubble in the chat — open Settings
  → Test API to re-check without sending a message.
- Running the binary by hand over SSH instead? Just export the variable first:
  `OPENROUTER_API_KEY=sk-or-... ./openrouter-chat` (needs a `QTFB_KEY` from
  AppLoad, so the menu is the normal route).

---

## Using the app

| Control | What it does |
|---|---|
| Header ⚙ | Open the settings menu |
| Header ◀ ▶ | Previous / next free model |
| Header model name | Open the free-model dropdown (tap a row to select, ◀ Prev / Next ▶ to page) |
| Header EXIT | Quit |
| Settings → API TEST | Test the key: `API OK — N free models` or the full error |
| Settings → MODEL | Current free model; tap to open the dropdown |
| Settings → TEXT SIZE | Tap left half (A−) / right half (A+) to shrink/enlarge |
| Settings → TEXT COLOUR | Tap to cycle Ink black → Navy → Sepia |
| Keyboard SHIFT | Tap once: next letter is capital (auto-releases) |
| Chat area, top half | Scroll transcript up |
| Chat area, bottom half | Scroll transcript down |
| Whiteboard pad | Write with the marker (flip the pen to erase) |
| **Send Ink-to-Text** | The selected vision model transcribes the pad, then answers it |
| **Send Image** | Sends the pad PNG inline as an `image_url` |
| **Keyboard** | Toggles the on-screen QWERTY keyboard over the pad |
| **Send Text** | Sends the typed prompt (keyboard mode) |
| Keyboard-mode footer: OCR / Image / Ink Pad / Send | Same sends without leaving the keyboard; Ink Pad returns to the pen pad |
| Keyboard SHIFT | One capital letter, then auto-releases |
| Keyboard `, . ?` / `- / :` | Multi-tap cycling: `,`→`.`→`?`→`!` and `-`→`/`→`:` |
| Settings footer: Test API / A− / A+ / Done | Test key, shrink/enlarge text, close |
| Dropdown footer: ◀ Prev / Next ▶ / Done | Page the free-model list, close |

Keyboard mode hides the pen pad; switch back with the same button to keep
writing. Pen ink is preserved across mode switches.

---

## Continuous integration

Every push to `main`, every pull request, and every `v*` tag builds the
AppLoad bundle through [.github/workflows/build.yml](.github/workflows/build.yml),
which runs `tools/build-appload.sh`:

1. Cross-compiles `openrouter-chat` for `aarch64-unknown-linux-musl` with
   `cargo zigbuild --release`.
2. Verifies the binary and assembles the bundle (binary + manifest + icon +
   `INSTALL.txt`).
3. Uploads `openrouter-chat-appload.tar.gz` as a workflow artifact; tag builds
   (`v*`) additionally attach it to a GitHub release.

To cut a release:

```sh
git tag v0.1.0
git push origin v0.1.0
```

---

## Environment variables

| Variable | Required | Meaning |
|---|---|---|
| `OPENROUTER_API_KEY` | yes | OpenRouter API key, read at startup |
| `QTFB_KEY` | set by AppLoad | Framebuffer handle; don't set manually |

## License

**AGPL-3.0** (see `LICENSE`). Vendored `vendor/rex` is MIT/Apache-2.0; bundled
fonts are OFL/MIT.
