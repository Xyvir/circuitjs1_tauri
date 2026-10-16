#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri desktop shell for CircuitJS1.
//!
//! This replaces the Electron wrapper in `app/`. The Java/GWT client is unchanged;
//! it keeps calling the same `$wnd.*` globals it always did. Those are supplied by
//! `bridge.js`, which is injected into every window as an initialization script,
//! and the work behind them happens here.
//!
//! All filesystem access lives in Rust rather than the JS `fs` plugin. That is
//! deliberate: a circuit handed to us on the command line was never picked through
//! a dialog, so the plugin's runtime path scope would reject it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

/// The query parameters the GWT client understands. Mirrors the `allowed` list in
/// the Electron `main.js`.
const ALLOWED_PARAMS: &[&str] = &[
    "startCircuit",
    "startLabel",
    "startCircuitLink",
    "positiveColor",
    "negativeColor",
    "neutralColor",
    "selectColor",
    "currentColor",
    "mouseMode",
    "running",
];

const HELP: &str = "\
Usage: CircuitJS1 [options] [file.cir]

Options:
  --help                   Show this help message
  --startCircuit=URL       Load circuit from URL
  --startLabel=LABEL       Label for the circuit
  --startCircuitLink=URL   Load circuit from Dropbox link
  --running=false          Start with simulation paused
  --mouseMode=MODE         Initial mouse mode
  --positiveColor=COLOR    Color for positive voltage
  --negativeColor=COLOR    Color for negative voltage
  --neutralColor=COLOR     Color for neutral/ground
  --selectColor=COLOR      Color for selected elements
  --currentColor=COLOR     Color for current flow";

/// Characters left unescaped when building the query string, matching the set
/// `encodeURIComponent` would leave alone.
const QUERY_ESCAPE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

static NEXT_WINDOW: AtomicU64 = AtomicU64::new(1);

/// Per-process state the window builder needs: the bridge source and the startup
/// query parameters, so `new_window` can reproduce the first window's behaviour.
struct ShellState {
    bridge: String,
    params: Vec<(String, String)>,
}

#[derive(Default)]
struct Args {
    file: Option<String>,
    params: Vec<(String, String)>,
    help: bool,
}

/// Parse process arguments the way the Electron shell did.
///
/// Unlike the original, a `--flag value` pair consumes its value, so a value can
/// never be mistaken for the circuit file to open.
fn parse_args(argv: &[String]) -> Args {
    let mut args = Args::default();
    let mut i = 1;

    while i < argv.len() {
        let arg = &argv[i];

        if arg == "--help" || arg == "-h" {
            args.help = true;
            i += 1;
            continue;
        }

        if let Some(rest) = arg.strip_prefix("--") {
            let (key, value) = match rest.split_once('=') {
                Some((k, v)) => (k.to_string(), v.to_string()),
                None => {
                    if i + 1 < argv.len() && !argv[i + 1].starts_with('-') {
                        let v = argv[i + 1].clone();
                        i += 1;
                        (rest.to_string(), v)
                    } else {
                        (rest.to_string(), "true".to_string())
                    }
                }
            };
            if ALLOWED_PARAMS.contains(&key.as_str()) {
                args.params.push((key, value));
            }
            i += 1;
            continue;
        }

        if args.file.is_none() {
            args.file = Some(arg.clone());
        }
        i += 1;
    }

    args
}

fn basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn encode(value: &str) -> String {
    utf8_percent_encode(value, QUERY_ESCAPE).to_string()
}

/// The page to load, with any recognised start-up parameters carried in the query
/// string (the GWT client reads them from `location.search` via QueryParameters).
fn app_url(params: &[(String, String)]) -> WebviewUrl {
    if params.is_empty() {
        return WebviewUrl::App(PathBuf::from("circuitjs.html"));
    }
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    WebviewUrl::App(PathBuf::from(format!("circuitjs.html?{query}")))
}

/// Compose the script injected into a window: the bridge, followed by the
/// start-circuit globals that the Electron preload derived from its `openFile`
/// query parameter. Values go through serde_json so they are safely escaped.
fn init_script(bridge: &str, file: Option<&str>) -> String {
    let (text, name) = match file {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(t) => (Some(t), Some(basename(path))),
            Err(e) => {
                eprintln!("circuitjs1: cannot open {}: {}", path, e);
                (None, None)
            }
        },
        None => (None, None),
    };

    let mut script = String::from(bridge);
    script.push_str("\nwindow.startCircuitText = ");
    script.push_str(&serde_json::to_string(&text).unwrap_or_else(|_| "null".into()));
    script.push_str(";\nwindow.startCircuitFileName = ");
    script.push_str(&serde_json::to_string(&name).unwrap_or_else(|_| "null".into()));
    script.push_str(";\n");
    script
}

fn create_window(app: &AppHandle, file: Option<&str>) -> tauri::Result<WebviewWindow> {
    let state = app.state::<ShellState>();
    let label = format!("circuitjs-{}", NEXT_WINDOW.fetch_add(1, Ordering::SeqCst));
    let script = init_script(&state.bridge, file);

    WebviewWindowBuilder::new(app, label, app_url(&state.params))
        .title("CircuitJS1")
        .inner_size(1200.0, 800.0)
        .initialization_script(script)
        .build()
}

// ---------------------------------------------------------------------------
// Commands called from bridge.js
// ---------------------------------------------------------------------------

#[tauri::command]
async fn pick_open_file(app: AppHandle) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_file(move |path| {
        let _ = tx.send(path);
    });
    rx.await
        .ok()
        .flatten()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
async fn pick_save_file(app: AppHandle, default_path: Option<String>) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut builder = app.dialog().file();
    if let Some(name) = default_path.as_deref() {
        builder = builder.set_file_name(name);
    }
    builder.save_file(move |path| {
        let _ = tx.send(path);
    });
    rx.await
        .ok()
        .flatten()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path, e))
}

#[tauri::command]
fn write_text_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| format!("{}: {}", path, e))
}

/// Write decoded binary data. Image export hands us a base64 PNG, and a text
/// write would corrupt it.
#[tauri::command]
fn write_base64_file(path: String, data: String) -> Result<(), String> {
    let bytes = BASE64
        .decode(data.trim())
        .map_err(|e| format!("invalid base64 data: {}", e))?;
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {}", path, e))
}

#[tauri::command]
async fn new_window(app: AppHandle) -> Result<(), String> {
    create_window(&app, None)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn toggle_devtools(window: WebviewWindow) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        window.open_devtools();
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let args = parse_args(&argv);

    if args.help {
        println!("{}", HELP);
        return;
    }

    let bridge = include_str!("bridge.js").to_string();
    let file = args.file.clone();

    tauri::Builder::default()
        // Registered first, as the plugin requires.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // A second launch (say, double-clicking another associated circuit)
            // forwards its arguments here instead of starting a new process.
            let args = parse_args(&argv);
            if let Err(e) = create_window(app, args.file.as_deref()) {
                eprintln!("circuitjs1: could not open new window: {}", e);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(ShellState {
            bridge,
            params: args.params,
        })
        .invoke_handler(tauri::generate_handler![
            pick_open_file,
            pick_save_file,
            read_text_file,
            write_text_file,
            write_base64_file,
            new_window,
            toggle_devtools
        ])
        .setup(move |app| {
            create_window(app.handle(), file.as_deref())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running CircuitJS1");
}
