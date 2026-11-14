# levain

levain is the shared native host of the Boulangerie libraries: baguette
(Elixir), chouquette (TS/JS) and briochette (Python). It renders an app's
view tree with GPUI, on Tauri, and talks to the app's language runtime over
a small C ABI. An app is written entirely in one language. levain is Rust
only; app developers never install it. Each language package ships a
prebuilt host built on it.

Docs: https://boulangerie.tznc.dev

This README is for people building a language host.

## Crates

| Crate | Kind | Contents |
| --- | --- | --- |
| `levain-abi` | lib | The C ABI: `HostApi`, `EventSink`, `ABI_VERSION`. No dependencies |
| `levain` | lib | Renderer, bridge, component sets, platform calls, app startup, mobile unpacking, the `Runtime` trait |
| `levain-pack` | bin | Packaging: precompiled host download, desktop bundles, Android and iOS projects |
| `levain-build` | lib | Build-script helpers for hosts: `build()` (tauri-build without a second Windows manifest) and `env_var()` (a variable or its `TAURI_` twin) |

`examples/echo` is a host with no language: a counter written in Rust
against the C ABI. It is levain's end-to-end test target and the smallest
host to copy from.

## The Runtime trait

A `Runtime` is the one seam between levain and a language. It has two methods.

- `locate(&self, &AppPaths) -> Result<App, String>` finds the app: next to the executable, in the bundle's
  resources (`AppPaths::resources`), or unpacked from the host library on
  mobile (`levain::mobile::unpack`). It returns an `App`: `root`, `name`, a
  `WindowConfig` (`title`, `width`, `height`; 480x360 by default) and `cli`
  (`Option<serde_json::Value>`, see [Command-line apps](#command-line-apps)).
  `WindowConfig::from_app_json` reads the `window` key of an `app.json`;
  `levain::cli_from_app_json` reads its `cli` key. On desktop `locate` runs
  before Tauri is built, so `AppPaths::data` is `None` there; it is set on
  phones, where `locate` runs inside Tauri's setup.
- `start(&self, App, &'static HostApi) -> Result<(), String>` starts the app on its own thread and returns. From then on the
  runtime talks to the host only through `api`. It never touches GPUI.
- If `locate` fails, window 0 shows "No app to run: ..." and `start` is not
  called.

## A minimal host

```rust
use levain::{App, AppPaths, Runtime, levain_abi::HostApi};

struct MyLang;

impl Runtime for MyLang {
    fn locate(&self, paths: &AppPaths) -> Result<App, String> { todo!() }
    fn start(&self, app: App, api: &'static HostApi) -> Result<(), String> { todo!() }
}

fn main() {
    levain::run(MyLang, tauri::generate_context!());
}
```

`levain` re-exports `levain_abi` and `tauri`. `run` takes the host's own
Tauri context, so the host crate owns its `tauri.conf.json`. On desktop,
`<host> --manifest [set]` prints the registered native components and exits.

The host crate needs:

- `tauri.conf.json` with `"app": {"windows": []}` (levain opens window 0
  itself) and `"build": {"frontendDist": []}`. See `examples/echo/` and
  `baguette_host`.
- `build.rs` that calls `tauri_build::try_build` with
  `WindowsAttributes::new_without_app_manifest()`. GPUI embeds its own
  Windows manifest; a second one fails the link.
- `capabilities/default.json` and an icon, as in `examples/echo/`.
- `tauri` with `default-features = false`, and `levain`.

On Android and iOS the host is a library. `baguette_host` is both: its
`lib.rs` has `#[cfg_attr(mobile, tauri::mobile_entry_point)] pub fn run()`
calling `levain::run`, its `main.rs` calls that for desktop, and its `[lib]`
has `crate-type = ["staticlib", "cdylib", "rlib"]`.

### Features

| Feature | Default | Effect |
| --- | --- | --- |
| `components-gpui-kit` | yes | Compiles in the `gpui-kit` native component set |
| `playwright` | no | Serves the windows to Playwright clients when `GPUI_PLAYWRIGHT` is set (a port, such as `9333`). `GPUI_PLAYWRIGHT_ENDPOINT_FILE` names a file the endpoint is written to. For test builds only |

A host forwards them (`components-gpui-kit = ["levain/components-gpui-kit"]`)
and depends on levain with `default-features = false`, as `baguette_host`
does.

## Wire format

Everything crossing the C ABI is UTF-8 JSON. `HostApi` has `abi_version`,
`ctx` and four functions, each taking `ctx`: `render(window, json, len)`,
`set_event_sink(EventSink)`, `check_components(json, len) -> bool` and
`call(json, len)`. `EventSink` is `{ctx, call(ctx, json, len)}`. Layouts are
tested in `levain-abi`. `ABI_VERSION` is 4 and changes whenever either
struct changes shape; the runtime should compare it with `api.abi_version`.
All functions are safe to call from any thread. The host calls the sink on
its UI thread, so do not block in it.

### Render: frames and patches

`api.render(ctx, window, json, len)` replaces what window `window` shows.
Window 0 is the main window. The JSON is one of three messages.

A frame, the whole view. `root` is required (`null` for no view): a message
without it is not a frame. The other keys are optional:

```json
{"root": <node>, "theme": {}, "lists": {"<id>": {"start": 0, "rows": [<node>]}},
 "keymap": [{"keys": "cmd-s", "action": "save", "context": "optional"}]}
```

A node is a string (text) or an element. `a` and `c` default to empty:

```json
{"t": "button", "a": {"phx-click": "inc", "class": "px-2"}, "c": ["+"]}
```

`t` is the tag, `a` the string attributes, `c` the children. A tag is a
built-in element or one from a compiled-in component set. Classes are
Tailwind-style. A `phx-*` attribute names the event to send back.

A patch, what changed since the last frame. `at` is a path of child indices
from the root; ops apply in order:

```json
{"patch": [<op>, ...], "lists": {}}
```

| `op` | Keys | Effect |
| --- | --- | --- |
| `replace` | `at`, `node` | Replaces the node at `at`. With `at: []` it replaces the root; `node: null` clears it |
| `attrs` | `at`, `a` | Replaces all attributes of the element at `at` |
| `splice` | `at`, `start`, `delete`, `c` | Removes `delete` children from `start` and inserts `c` there |

If an op does not fit the host's tree, the host sends a `resync` event and
the runtime must send a whole frame.

Rows for a virtualized `<list>`, sent in reply to a `rows` event:

```json
{"rows": {"id": "<id>", "start": 40, "rows": [<node>]}}
```

### Events

The host calls the runtime's sink with one JSON object per event. Every
event has `type`; `window` is the window number.

| `type` | Other keys |
| --- | --- |
| `click`, `change`, `submit`, `keydown`, `pick` | `window`, `event` (the `phx-*` name), `values` (object) |
| `drag` | `window`, `event`, `values`, `dx`, `dy` |
| `rows` | `window`, `event`, `id`, `start`, `end`, `visible` (`[first, last]`) |
| `action` | `window`, `action` (a keymap action) |
| `appearance` | `window`, `appearance` (`light` or `dark`) |
| `resync` | `window` |
| `menu` | `window` (always 0), `event` |
| `reply` | `ref`, `result`, and `error` on failure |

### Platform calls

`api.call` takes `{"call": "<name>", "ref": <n>, "args": {...}}`. The host
runs it on its UI thread and sends a `reply` event with the same `ref`:
`{"type": "reply", "ref": n, "result": ...}`. A failed call has `"result":
null` and `"error": "<call>: <message>"`. An unknown call is an error reply.

| Call | Args | Result |
| --- | --- | --- |
| `clipboard.read` | | text or `null` |
| `clipboard.write` | `text` | `null` |
| `app.appearance` | | `"light"` or `"dark"` |
| `app.open_url` | `url` | `null` |
| `menu.set` | `menus`: `[{title, items: [{id, label, keys?} \| {separator: true}]}]` | `null`. Desktop only; a no-op on phones |
| `window.open` | `window`, `title?`, `width?`, `height?` (default 800x600) | `null` |
| `window.close` | `window` | `null` |
| `window.set_title` | `window`, `title` | `null` |
| `window.set_bounds` | `window`, `x?`, `y?`, `width?`, `height?` | `null` |
| `window.info` | `window` | `{title, x, y, width, height}`, or `null` for no such window |
| `cli.matches` | | The parsed command line, as `tauri-plugin-cli` gives it (see [Command-line apps](#command-line-apps)). An error if the app declares no `cli` |
| `app.exit` | `code?` (integer, default 0) | `null`, sent before the process exits with `code` |

### Command-line apps

An app can also be a command-line tool, built on Tauri's CLI plugin
(`tauri-plugin-cli`). The app's own code decides, per run, whether to work
headless (no window: do the work, print, exit with a status) or open windows.

Declare the CLI in the app's `app.json` under `cli`, in the schema Tauri's
CLI plugin takes under `plugins.cli` in `tauri.conf.json` (`description`,
`args`, `subcommands`, ...), and return it as `App::cli`
(`levain::cli_from_app_json(bytes)`):

```json
{"cli": {"description": "Adds numbers", "subcommands": {"sum": {"args": [
  {"name": "a", "index": 1, "takesValue": true, "required": true},
  {"name": "b", "index": 2, "takesValue": true, "required": true},
  {"name": "json"}]}}}}
```

`levain::run` puts it in the Tauri context's plugin config and registers the
plugin before the app is built. The app then calls:

- `cli.matches`, with no args. The result is the plugin's matches:
  `{"args": {"<name>": {"value": ..., "occurrences": n}}, "subcommand":
  {"name": "sum", "matches": {...}} | null}`. `value` is `true`/`false` for a
  flag, a string for an option or positional, an array for one that takes
  several, `null` when absent. `--help` is a match, not an exit: `args.help.value`
  is the help text. Arguments the CLI does not accept are an error reply
  (`cli.matches: failed to parse arguments: ...`, with clap's message). An
  app that declared no `cli` gets `cli.matches: the app declares no cli`.
- `app.exit` with `{"code": n}` (default 0). The reply is sent first, then
  the process exits with `n` through Tauri; anything the app printed must be
  flushed by the app before it makes the call (the host flushes its own
  stdout and stderr). A `code` that is not an integer is an error reply and
  the process stays.

Window 0 is not opened at startup when the app declares a `cli`: it opens,
with the app's `title`, `width` and `height`, the first time the app renders
to window 0 or calls `window.open` for it (whatever its args). Renders wait
for it in the queue for windows not open yet, so none is lost. A headless
run, which renders nothing, never creates a window. It must end itself with
`app.exit`: with no window open, nothing else ends the process. An app that
declares no `cli` opens window 0 at once, as ever, and the "No app to run"
notice still appears when `locate` fails.

Limits: desktop only. On Android and iOS the plugin is not built, `App::cli`
is ignored, window 0 opens at once and `cli.matches` is the "declares no cli"
error. A `cli` that does not fit the plugin's schema fails the host's
startup. On Linux a headless run still needs a display (X or Wayland, or
`xvfb-run`): Tauri initialises GTK at startup and fails without one, with
"Failed to initialize GTK".

### Component sets

A runtime built against native component manifests should confirm they
match the host's before its first render. It calls `api.check_components`
with a JSON object of set name to manifest hash:

```json
{"gpui-kit": "ad5978e625448927"}
```

If a set is in both with different hashes, the call returns `false`, window
0 shows why and later renders are ignored. A set the host lacks is logged,
and its tags draw as `<div>`s. `<host> --manifest [set]` prints the
manifests, with their `hash`.

### Conformance

`fixtures/` is the conformance set. A language's implementation should pass
all of it.

| File | Holds |
| --- | --- |
| `patches.json` | `{name, old, new, patch}`: applying `patch` to `old` must give `new` |
| `events.json` | `{name, build, args, json}`: the event `json` for each type in the table above |
| `calls.json` | `{name, json}`: a platform call request; with `reply`, the request is invalid and `reply` is the answer (`raw` is text that is not JSON for a request) |
| `components.json` | A component manifest, as `--manifest` prints it |

levain's own tests run the first two against `patch.rs` and `events.rs`.

## Mobile

On Android and iOS the app ships inside the host library. Set
`LEVAIN_APP_ARCHIVE` to a `.tar.gz` of the app when building the host.
`levain/build.rs` embeds it (a `TAURI_`-prefixed name is read too, as Tauri's
iOS build passes on only those). In `locate`, call:

```rust
let root = levain::mobile::unpack(&paths.data.clone().ok_or("no data dir")?, |_| Ok(()))?;
```

`unpack` extracts into `<data>/app-<hash>/` once per app build and
removes earlier `app-*` directories. The closure runs on the unpacked
files before they are moved into place (on Android: linking native
libraries). Without the archive it returns an error. On Android,
`levain::mobile::native_lib_dir()` returns the library directory.
`levain::mobile` exists only on mobile targets and in tests.

## levain-pack

A binary that knows nothing about the language. Language packages call it.

| Command | Does |
| --- | --- |
| `target` | Prints the desktop target name: `x86_64-linux`, `aarch64-macos`, `x86_64-windows`, ... |
| `fetch <base-url> <asset> --sha256 <sums> --cache <dir>` | Downloads a precompiled asset, checks it against a `sha256sum` file and prints its directory |
| `install <host> <dest>` | Copies the host to `dest`, executable |
| `bundle --project <dir> --host <exe> --host-name <name> --name <app> --version <v> --identifier <id> --bundles <a,b> --target-dir <dir> --out <dir> [--resource <from>=<to>]... [--config <extra>]` | Desktop bundles (deb, AppImage, dmg, msi, ...) with the Tauri bundler around a built host |
| `ios-crate --host-crate <name> --host-lib <lib name> --host-path <dir> --out <dir> [--feature <f>]... [--default-feature <f>]... [--name <crate>] [--product-name <n>] [--version <v>] [--identifier <id>]` | Writes the iOS wrapper crate: a `staticlib` re-exporting the host's `run`. Every host needs one beside it |
| `init <android\|ios> --project <dir> --config <json> [--signing-env <PREFIX>] [--ldflags <flags>] [--env K=V]...` | Generates the Tauri mobile project, patched for levain |
| `build <android\|ios> --project <dir> --config <json> [--product <ext>=<path>]... [--env K=V]... [-- <tauri args>]` | Builds it and copies each output with extension `<ext>` (such as `apk`) to its path |

Unknown options are an error. Tagged releases attach `levain-pack` binaries
for six desktop targets, with SHA-256s.

## Depending on levain

`levain` is not on crates.io: crates.io rejects git dependencies, and levain
has three (`tauri-plugin-gpui`, `gpui-playwright`, the patched gpui-kit
fork). Pin it by git revision, as above. The release workflow publishes
`levain-abi` and `levain-pack`.

Cargo ignores `[patch]` in dependencies, so the host's workspace repeats
levain's `[patch.crates-io]` block. Copy it from levain's `Cargo.toml` at the
same revision:

```toml
[patch.crates-io]
gpui-kit = { git = "https://github.com/Ziinc/gpui-kit", rev = "7ea1125954e6701d849fd448aab08ab2ac63d07f" }
gpui-base = { git = "https://github.com/Ziinc/gpui-kit", rev = "7ea1125954e6701d849fd448aab08ab2ac63d07f" }
gpui-component = { git = "https://github.com/Ziinc/gpui-kit", rev = "7ea1125954e6701d849fd448aab08ab2ac63d07f" }
gpui-component-macros = { git = "https://github.com/Ziinc/gpui-kit", rev = "7ea1125954e6701d849fd448aab08ab2ac63d07f" }
gpui-kit-assets = { git = "https://github.com/Ziinc/gpui-kit", rev = "7ea1125954e6701d849fd448aab08ab2ac63d07f" }
```

The fork is gpui-kit 0.7.3 plus the painted-text observation the
`playwright` feature reads. levain records the GPUI and gpui-kit versions
from its own `Cargo.lock` in each component manifest.

## Tests

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p levain --all-targets --features playwright -- -D warnings
```

On Linux, GPUI and Tauri need system libraries; see
`.github/workflows/ci.yml`. CI tests on Linux, macOS and Windows and builds
for iOS and Android.

### Linting

CI's `lint` job also runs [ast-grep](https://ast-grep.github.io) rules
(`lint/ast-grep`, configured by `sgconfig.yml`), [typos](https://github.com/crate-ci/typos),
[cargo-machete](https://github.com/bnjbvr/cargo-machete) and
[cargo-deny](https://github.com/EmbarkStudios/cargo-deny) (`deny.toml`):

```sh
ast-grep test   # the rules' own tests; `-U` updates their snapshots
ast-grep scan
typos
cargo machete
cargo deny --locked check
```

`e2e/` drives the echo host with Playwright through gpui-playwright (Linux
CI, under `xvfb-run`):

```sh
cargo build -p levain-echo --features playwright
cd e2e && npm install && npx playwright test
npm run test:cli   # the headless path: `levain-echo sum 2 3`, no window
```

## Licence

MIT.
