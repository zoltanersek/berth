mod api;
mod auth;
mod sse;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;

use tiny_http::{Header, Method, Request, Response, Server};

use api::Route;

/// The single-page dashboard UI. `__BERTH_TOKEN__` is replaced with the live
/// session token before the page is served.
const INDEX_HTML: &str = include_str!("index.html");

/// Shared, read-only state passed to every request handler.
struct AppState {
    root: PathBuf,
    token: String,
}

/// Serve the dashboard on `127.0.0.1`, blocking until the process is killed.
pub fn serve(dir: &Path, port: Option<u16>, no_open: bool) -> Result<(), String> {
    let token = auth::generate_token()?;

    let addr = format!("127.0.0.1:{}", port.unwrap_or(0));
    let server =
        Server::http(&addr).map_err(|e| format!("Failed to start dashboard server: {e}"))?;

    // Bind to port 0 by default and read the assigned port back — no separate
    // free-port dance, and no window between choosing and binding it.
    let bound = server
        .server_addr()
        .to_ip()
        .ok_or("Failed to determine the bound address")?;
    let url = format!("http://127.0.0.1:{}", bound.port());

    println!("Berth dashboard running at {url}");
    println!("Press Ctrl-C to stop.");

    if !no_open {
        open_browser(&url);
    }

    let state = Arc::new(AppState {
        root: dir.to_path_buf(),
        token,
    });

    // One thread per request. Log streams block their handler for the life of
    // the stream, so a fixed-size pool could starve; this is a single-user
    // local tool, so unbounded spawning is fine.
    loop {
        match server.recv() {
            Ok(request) => {
                let state = Arc::clone(&state);
                thread::spawn(move || handle(request, &state));
            }
            Err(e) => eprintln!("dashboard: connection error: {e}"),
        }
    }
}

fn handle(request: Request, state: &AppState) {
    let method = method_str(request.method());
    let url = request.url().to_string();
    let route = api::parse_route(method, &url);

    // The index page and 404s carry no secret, so they skip the token check;
    // every other route is token-gated.
    let needs_auth = !matches!(route, Route::Index | Route::NotFound);
    if needs_auth && !authorized(&request, &url, &state.token) {
        return respond_forbidden(request);
    }

    match route {
        Route::Index => {
            // Still refuse a rebound (non-loopback) Host, even without a token.
            if host_is_loopback(&request) {
                let html = INDEX_HTML.replace("__BERTH_TOKEN__", &state.token);
                let response = Response::from_string(html)
                    .with_header(content_type("text/html; charset=utf-8"));
                let _ = request.respond(response);
            } else {
                respond_forbidden(request);
            }
        }
        Route::NotFound => {
            let _ = request.respond(Response::from_string("not found").with_status_code(404));
        }
        Route::Snapshot => {
            let (code, body) = api::snapshot(state);
            respond_json(request, code, body);
        }
        Route::Services(name) => {
            let (code, body) = api::services(state, &name);
            respond_json(request, code, body);
        }
        Route::Logs(name) => sse::stream_logs(request, state, &name),
        Route::Action(name, kind) => {
            let mut request = request;
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            let (code, response) = api::action(state, &name, kind, &body);
            respond_json(request, code, response);
        }
    }
}

fn authorized(request: &Request, url: &str, expected: &str) -> bool {
    let host = header_value(request.headers(), "Host");
    let origin = header_value(request.headers(), "Origin");
    // `EventSource` can't set headers, so the log stream passes its token in
    // the query string; POSTs use the `X-Berth-Token` header.
    let token = header_value(request.headers(), "X-Berth-Token")
        .or_else(|| auth::query_param(url, "token").map(str::to_string));

    auth::authorize(
        host.as_deref(),
        origin.as_deref(),
        token.as_deref(),
        expected,
    )
}

fn host_is_loopback(request: &Request) -> bool {
    header_value(request.headers(), "Host")
        .as_deref()
        .map(auth::is_loopback_host)
        .unwrap_or(false)
}

fn header_value(headers: &[Header], name: &'static str) -> Option<String> {
    headers
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str().to_string())
}

fn method_str(method: &Method) -> &'static str {
    match method {
        Method::Get => "GET",
        Method::Post => "POST",
        _ => "",
    }
}

fn content_type(value: &'static str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], value.as_bytes()).expect("valid content-type header")
}

fn respond_json(request: Request, code: u16, body: String) {
    let response = Response::from_string(body)
        .with_status_code(code)
        .with_header(content_type("application/json"));
    let _ = request.respond(response);
}

fn respond_forbidden(request: Request) {
    let _ = request.respond(Response::from_string("forbidden").with_status_code(403));
}

/// Best-effort: open the dashboard in the user's default browser.
fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = Command::new(opener).arg(url).spawn();
}
