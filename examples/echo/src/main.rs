//! A host with no language runtime: a counter written in Rust against the
//! C ABI, for levain's own end-to-end test. It is also a command-line tool:
//! `levain-echo sum 2 3` prints `5` and exits without opening a window, and
//! `levain-echo` alone opens the counter.

use std::ffi::c_void;
use std::sync::mpsc;

use levain::levain_abi::{EventSink, HostApi};
use levain::{App, AppPaths, Runtime, WindowConfig, cli_from_app_json};
use serde_json::{Value, json};

struct Echo;

impl Runtime for Echo {
    fn locate(&self, _: &AppPaths) -> Result<App, String> {
        let json = include_bytes!("../app.json");
        Ok(App {
            root: ".".into(),
            name: "echo".into(),
            window: WindowConfig::from_app_json(json),
            cli: cli_from_app_json(json),
        })
    }

    fn start(&self, _: App, api: &'static HostApi) -> Result<(), String> {
        std::thread::spawn(move || run(api));
        Ok(())
    }
}

extern "C" fn on_event(ctx: *mut c_void, json: *const u8, len: usize) {
    // SAFETY: `ctx` is the leaked sender `run` installed; the host passes a
    // valid buffer for the call.
    let (tx, bytes) = unsafe {
        (
            &*(ctx as *const mpsc::Sender<Vec<u8>>),
            std::slice::from_raw_parts(json, len),
        )
    };
    let _ = tx.send(bytes.to_vec());
}

fn render(api: &HostApi, count: i64) {
    let button =
        |event: &str, label: &str| json!({"t": "button", "a": {"phx-click": event}, "c": [label]});
    let frame = json!({"root": {
        "t": "div",
        "a": {"class": "size-full flex flex-col items-center justify-center gap-4"},
        "c": [
            {"t": "p", "c": [format!("Count: {count}")]},
            {"t": "div", "a": {"class": "flex gap-2"}, "c": [button("dec", "-"), button("inc", "+")]},
        ],
    }});
    let bytes = frame.to_string().into_bytes();
    (api.render)(api.ctx, 0, bytes.as_ptr(), bytes.len());
}

/// What the command line asks for.
#[derive(Debug, PartialEq)]
enum Command {
    /// No subcommand: the counter window.
    Window,
    /// Print, then exit with `code`; nothing is rendered.
    Exit {
        code: i32,
        stdout: Option<String>,
        stderr: Option<String>,
    },
}

/// The command in the reply to `cli.matches`.
fn command(reply: &Value) -> Command {
    let exit = |code, stdout: Option<String>, stderr: Option<String>| Command::Exit {
        code,
        stdout,
        stderr,
    };
    if let Some(error) = reply["error"].as_str() {
        return exit(2, None, Some(error.to_owned()));
    }
    let matches = &reply["result"];
    // `--help` is a match carrying the help text.
    if let Some(help) = matches["args"]["help"]["value"].as_str() {
        return exit(0, Some(help.trim_end().to_owned()), None);
    }
    let sub = &matches["subcommand"];
    if sub["name"] != "sum" {
        return Command::Window;
    }
    let args = &sub["matches"]["args"];
    let number = |name: &str| {
        let text = args[name]["value"].as_str().unwrap_or_default();
        text.parse::<f64>()
            .map_err(|_| format!("levain-echo: not a number: {text}"))
    };
    match (number("a"), number("b")) {
        (Ok(a), Ok(b)) if args["json"]["value"] == true => {
            exit(0, Some(json!({"sum": a + b}).to_string()), None)
        }
        (Ok(a), Ok(b)) => exit(0, Some((a + b).to_string()), None),
        (Err(error), _) | (_, Err(error)) => exit(2, None, Some(error)),
    }
}

fn call(api: &HostApi, reference: u64, name: &str, args: Value) {
    let bytes = json!({"call": name, "ref": reference, "args": args})
        .to_string()
        .into_bytes();
    (api.call)(api.ctx, bytes.as_ptr(), bytes.len());
}

fn run(api: &'static HostApi) {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let ctx = Box::into_raw(Box::new(tx)) as *mut c_void;
    (api.set_event_sink)(
        api.ctx,
        EventSink {
            ctx,
            call: on_event,
        },
    );
    // The app decides, per run, between a window and a command: nothing is
    // rendered until the command line says so.
    call(api, 1, "cli.matches", json!({}));
    let mut count = 0;
    while let Ok(bytes) = rx.recv() {
        let Ok(event) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if event["type"] == "reply" && event["ref"] == 1 {
            match command(&event) {
                Command::Window => render(api, count),
                Command::Exit {
                    code,
                    stdout,
                    stderr,
                } => {
                    if let Some(text) = stdout {
                        println!("{text}");
                    }
                    if let Some(text) = stderr {
                        eprintln!("{text}");
                    }
                    call(api, 2, "app.exit", json!({"code": code}));
                }
            }
            continue;
        }
        if event["type"] != "click" {
            continue;
        }
        match event["event"].as_str() {
            Some("inc") => count += 1,
            Some("dec") => count -= 1,
            _ => continue,
        }
        render(api, count);
    }
}

fn main() {
    levain::run(Echo, tauri::generate_context!());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    static FRAMES: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    static SINK: Mutex<Option<EventSink>> = Mutex::new(None);

    extern "C" fn render_into_frames(_: *mut c_void, window: u32, json: *const u8, len: usize) {
        // SAFETY: `render` passes a buffer valid for `len` bytes for this call.
        let json = unsafe { std::slice::from_raw_parts(json, len) };
        let mut frame: Value = serde_json::from_slice(json).unwrap();
        frame["window"] = window.into();
        FRAMES.lock().unwrap().push(frame);
    }

    extern "C" fn keep_sink(_: *mut c_void, sink: EventSink) {
        *SINK.lock().unwrap() = Some(sink);
    }

    extern "C" fn accept(_: *mut c_void, _: *const u8, _: usize) -> bool {
        true
    }

    /// The host's answer to `cli.matches`: no subcommand, so the window.
    extern "C" fn answer_cli_matches(_: *mut c_void, json: *const u8, len: usize) {
        // SAFETY: `call` passes a buffer valid for `len` bytes for this call.
        let call: Value =
            serde_json::from_slice(unsafe { std::slice::from_raw_parts(json, len) }).unwrap();
        assert_eq!(call["call"], "cli.matches");
        send(
            &json!({"type": "reply", "ref": call["ref"], "result": {"args": {}, "subcommand": null}})
                .to_string(),
        );
    }

    fn send(event: &str) {
        let sink = SINK
            .lock()
            .unwrap()
            .expect("the runtime installed its sink");
        (sink.call)(sink.ctx, event.as_ptr(), event.len());
    }

    /// The text of the frame's count label.
    fn count(frame: &Value) -> String {
        frame["root"]["c"][0]["c"][0].as_str().unwrap().to_owned()
    }

    fn wait_for_frames(n: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let frames = FRAMES.lock().unwrap().clone();
            if frames.len() >= n || Instant::now() > deadline {
                return frames;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_counter_counts_clicks_and_ignores_everything_else() {
        let app = Echo.locate(&AppPaths::default()).unwrap();
        assert_eq!(
            (app.name.as_str(), app.window.title.as_deref()),
            ("echo", Some("Echo"))
        );
        // It declares the `sum` subcommand.
        assert!(app.cli.as_ref().unwrap()["subcommands"]["sum"].is_object());
        assert_eq!((app.window.width, app.window.height), (320.0, 200.0));

        let api: &'static HostApi = Box::leak(Box::new(HostApi {
            abi_version: levain::levain_abi::ABI_VERSION,
            ctx: std::ptr::null_mut(),
            render: render_into_frames,
            set_event_sink: keep_sink,
            check_components: accept,
            call: answer_cli_matches,
        }));
        Echo.start(app, api).unwrap();

        let first = wait_for_frames(1);
        assert_eq!(count(&first[0]), "Count: 0");
        assert_eq!(first[0]["window"], 0);
        // The buttons send the app's events on click.
        let buttons = &first[0]["root"]["c"][1]["c"];
        assert_eq!(buttons[0]["a"]["phx-click"], "dec");
        assert_eq!(buttons[1]["a"]["phx-click"], "inc");

        let click =
            |event: &str| format!(r#"{{"type": "click", "window": 0, "event": "{event}"}}"#);
        send(&click("inc"));
        send(&click("inc"));
        // None of these is a click on `inc` or `dec`, so none renders.
        send("not json");
        send(r#"{"type": "change", "window": 0, "event": "inc"}"#);
        send(&click("other"));
        send(r#"{"type": "click", "window": 0}"#);
        send(&click("dec"));
        let frames = wait_for_frames(4);
        let counts: Vec<String> = frames.iter().map(count).collect();
        assert_eq!(counts, ["Count: 0", "Count: 1", "Count: 2", "Count: 1"]);
    }

    fn reply(result: Value) -> Value {
        json!({"type": "reply", "ref": 1, "result": result})
    }

    fn sum(a: &str, b: &str, json_flag: bool) -> Value {
        reply(
            json!({"args": {}, "subcommand": {"name": "sum", "matches": {"args": {
            "a": {"value": a, "occurrences": 1},
            "b": {"value": b, "occurrences": 1},
            "json": {"value": json_flag, "occurrences": u8::from(json_flag)},
        }, "subcommand": null}}}),
        )
    }

    fn done(code: i32, stdout: Option<&str>, stderr: Option<&str>) -> Command {
        Command::Exit {
            code,
            stdout: stdout.map(Into::into),
            stderr: stderr.map(Into::into),
        }
    }

    #[test]
    fn sum_prints_the_total_and_exits_zero() {
        assert_eq!(command(&sum("2", "3", false)), done(0, Some("5"), None));
        assert_eq!(command(&sum("1.5", "1", false)), done(0, Some("2.5"), None));
        assert_eq!(
            command(&sum("2", "3", true)),
            done(0, Some(r#"{"sum":5.0}"#), None)
        );
    }

    #[test]
    fn sum_of_what_is_not_a_number_fails_with_code_two() {
        assert_eq!(
            command(&sum("x", "3", false)),
            done(2, None, Some("levain-echo: not a number: x"))
        );
        assert_eq!(
            command(&sum("2", "y", false)),
            done(2, None, Some("levain-echo: not a number: y"))
        );
    }

    #[test]
    fn no_subcommand_is_the_window_and_a_parse_error_or_help_is_a_command() {
        assert_eq!(
            command(&reply(json!({"args": {}, "subcommand": null}))),
            Command::Window
        );
        let other = json!({"args": {}, "subcommand": {"name": "other", "matches": {}}});
        assert_eq!(command(&reply(other)), Command::Window);
        let error = json!({"type": "reply", "ref": 1, "result": null,
            "error": "cli.matches: failed to parse arguments: nope"});
        assert_eq!(
            command(&error),
            done(
                2,
                None,
                Some("cli.matches: failed to parse arguments: nope")
            )
        );
        let help = reply(json!({"args": {"help": {"value": "Usage: x\n", "occurrences": 0}}}));
        assert_eq!(command(&help), done(0, Some("Usage: x"), None));
    }
}
