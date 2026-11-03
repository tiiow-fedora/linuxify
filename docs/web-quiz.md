# The website quiz in a window

Linuxify can show a distro questionnaire that lives on a website inside its own window, instead of
the built-in questions. This page is the contract between that website and the app.

## Turning it on

The app needs to know where the page is. In order of priority:

1. `linuxify choose --web --quiz-url https://example.org/quiz/`
2. the environment variable `LINUXIFY_QUIZ_URL`
3. a `quiz.json` in `%LOCALAPPDATA%\Linuxify\`, or `data\quiz.json` next to the exe:

   ```json
   { "url": "https://example.org/quiz/" }
   ```
   or, to ship the page with the app (served from the folder holding that file):
   ```json
   { "file": "quiz/index.html" }
   ```

Only `https://` addresses are accepted (`http://` only for `localhost`, for development). With none of
these set, the window shows the built-in quiz exactly as before.

The web view is compiled in with `cargo build --release --features webview` (it needs the Microsoft
Edge WebView2 runtime, which Windows 11 and current Windows 10 have). Builds without it say so and
keep using the built-in quiz.

## What the page sends back

When the visitor finishes, the page sends **one JSON message**, in either way:

* `window.ipc.postMessage(JSON.stringify(result))`, or
* navigate to any URL that has `linuxify_result=<base64url of the JSON>` in its query or `#fragment`
  (the app intercepts it; nothing is loaded).

`result` is one of:

```json
{ "version": 1, "answers": { "experience": "never", "gaming": "lots", ... } }
```
All 14 questions, each with one of its option keys. `linuxify quiz-spec` prints the exact ids, keys
and wording. The app combines these answers with the PC scan (graphics card, RAM, Secure Boot, games)
to rank the distros.

```json
{ "version": 1, "distro": "linux-mint" }
```
The page already chose. The id must be one in `data/distros.json`. The app remembers it as the pick
for `try` and `install`.

Anything else is refused with an explanation: unknown fields, a missing or extra question, an
unknown option, a distro the app doesn't know, a message over 16 KB.

## Safety rules the window enforces

* The page can only send the JSON above. It cannot run commands, read files or choose paths.
* Links on the quiz's own site stay in the window. Other `https://` links open in the normal
  browser. `file:`, `javascript:`, `data:` and app-protocol links are blocked. Pop-ups, downloads,
  camera, microphone and location requests are denied.
* A local quiz (`"file"`) is served by the app from its own folder, read-only, for a short list of
  file types (html, js, css, json, images, fonts), and cannot reach outside that folder.
* The window runs as a separate process, so a crashing page cannot take the app down.

## Testing a page without Windows

`linuxify choose --quiz-result result.json` accepts the JSON from a file, and runs the same checks.
