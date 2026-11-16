//! Native part of the `@lsdocs/rst` Node.js package.
//!
//! The JavaScript API lives in `index.js`; this addon provides what it
//! wraps. Values cross as JSON strings: trees in the form of the `serde`
//! feature of `lsdocs-rst`.
//!
//! JavaScript runs on one thread, and a JS directive's context
//! (`parseContent`, ...) needs the parser's `DirectiveContext`, which
//! borrows the parse. So `parse()` runs the parser on a scoped worker
//! thread and the JS thread serves it: when the parser reaches a JS
//! directive, role or file loader, the worker sends a call to the JS
//! thread, which runs the `dispatch` function and sends the answer back.
//! A directive's context calls `request()`, which sends the parse it wants
//! to the worker (blocked in that directive) and serves further JS calls,
//! such as a JS directive in the content being parsed, until the answer
//! comes. The two threads take turns, like one call stack.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use lsdocs_rst::{
    Directive, DirectiveContext, DirectiveError, DirectiveSpec, Element, FileLoader, Node,
    OptionKind, OptionValue, Parser, Role, RoleContext, Settings, XmlOptions,
};
use napi::bindgen_prelude::{Function, FunctionRef};
use napi::{Env, Error, Result, Status};
use napi_derive::napi;
use serde_json::{Value, json};

fn to_napi(message: impl std::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, message.to_string())
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(to_napi)
}

fn from_json<T: serde::de::DeserializeOwned>(json: &str) -> Result<T> {
    serde_json::from_str(json).map_err(to_napi)
}

// -- the bridge --------------------------------------------------------------------

/// A parse asked for by a directive's context.
enum Request {
    ParseContent(Element),
    ParseInline(String),
    Message(i64, String),
}

/// From the parsing thread to the JS thread.
enum ToJs {
    /// Run `dispatch` with this JSON and send back its answer.
    Call(String),
    /// The answer to a `Request`, as JSON.
    Reply(String),
    /// The parse is done.
    Done(Box<(Element, Vec<Element>)>),
}

/// From the JS thread to the parsing thread.
enum ToParser {
    /// What `dispatch` returned.
    Answer(String),
    Request(Request),
}

/// The parsing thread's end.
struct ParserSide {
    to_js: Sender<ToJs>,
    from_js: Mutex<Receiver<ToParser>>,
}

impl ParserSide {
    /// Calls `dispatch` with `call`, serving the requests of a directive's
    /// context (`cx`) meanwhile. `None` if the JS side has gone.
    fn call(&self, call: Value, mut cx: Option<&mut DirectiveContext<'_>>) -> Option<Value> {
        self.to_js.send(ToJs::Call(call.to_string())).ok()?;
        loop {
            let message = self
                .from_js
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv()
                .ok()?;
            match message {
                ToParser::Answer(answer) => return serde_json::from_str(&answer).ok(),
                ToParser::Request(request) => {
                    let reply = match cx.as_deref_mut() {
                        Some(cx) => answer(cx, request),
                        None => json!({"error": "only a directive's context can parse"}),
                    };
                    self.to_js.send(ToJs::Reply(reply.to_string())).ok()?;
                }
            }
        }
    }
}

fn answer(cx: &mut DirectiveContext<'_>, request: Request) -> Value {
    let result = match request {
        Request::ParseContent(container) => serde_json::to_value(cx.parse_content(container)),
        Request::ParseInline(text) => serde_json::to_value(cx.parse_inline(&text)),
        Request::Message(level, text) => serde_json::to_value(cx.message(level, &text)),
    };
    match result {
        Ok(value) => json!({ "ok": value }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// The JS thread's end.
struct JsSide {
    dispatch: FunctionRef<String, String>,
    to_parser: Sender<ToParser>,
    from_parser: Receiver<ToJs>,
}

/// What ended a wait on the JS thread.
enum Woken {
    Reply(String),
    Done(Box<(Element, Vec<Element>)>),
}

impl JsSide {
    /// Serves calls from the parsing thread until a reply or the end of
    /// the parse.
    fn serve(&self, env: &Env) -> Result<Woken> {
        loop {
            let message = self
                .from_parser
                .recv()
                .map_err(|_| to_napi("the parser stopped unexpectedly"))?;
            match message {
                ToJs::Call(call) => {
                    let answer = self.dispatch.borrow_back(env)?.call(call);
                    // The JS wrapper catches everything; should dispatch
                    // throw anyway, the parser still needs an answer.
                    let answer = answer.unwrap_or_else(|_| "[\"throw\"]".to_owned());
                    self.to_parser
                        .send(ToParser::Answer(answer))
                        .map_err(|_| to_napi("the parser stopped unexpectedly"))?;
                }
                ToJs::Reply(reply) => return Ok(Woken::Reply(reply)),
                ToJs::Done(done) => return Ok(Woken::Done(done)),
            }
        }
    }
}

thread_local! {
    /// The parses running on this (the JS) thread, innermost last: a
    /// directive may parse a document of its own.
    static PARSES: RefCell<Vec<Rc<JsSide>>> = const { RefCell::new(Vec::new()) };
}

/// Removes the innermost parse when dropped.
struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        PARSES.with(|p| p.borrow_mut().pop());
    }
}

// -- extensions --------------------------------------------------------------------

fn option_json(value: &OptionValue) -> Value {
    match value {
        OptionValue::Flag => Value::Null,
        OptionValue::Text(s) => s.clone().into(),
        OptionValue::Int(n) => (*n).into(),
        OptionValue::List(l) => l.clone().into(),
        _ => Value::Null,
    }
}

fn options_json(options: &[(String, OptionValue)]) -> Value {
    Value::Object(
        options
            .iter()
            .map(|(name, value)| (name.clone(), option_json(value)))
            .collect(),
    )
}

/// An outcome from the JS wrapper: `["ok", nodes]`, `["error", level,
/// message]` or `["throw"]` (the exception is kept on the JS side).
fn outcome(answer: Option<Value>, what: &str) -> std::result::Result<Vec<Node>, DirectiveError> {
    let failed = || DirectiveError::error(format!("{what} raised an exception."));
    let Some(Value::Array(items)) = answer else {
        return Err(failed());
    };
    match items.first().and_then(Value::as_str) {
        Some("ok") => items
            .get(1)
            .cloned()
            .and_then(|nodes| serde_json::from_value(nodes).ok())
            .ok_or_else(|| DirectiveError::error(format!("{what} returned invalid nodes."))),
        Some("error") => Err(DirectiveError {
            level: items.get(1).and_then(Value::as_i64).unwrap_or(3),
            message: items
                .get(2)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }),
        _ => Err(failed()),
    }
}

struct JsDirective {
    spec: DirectiveSpec,
    bridge: Arc<ParserSide>,
}

impl Directive for JsDirective {
    fn spec(&self) -> DirectiveSpec {
        self.spec.clone()
    }

    fn run(&self, cx: &mut DirectiveContext<'_>) -> std::result::Result<Vec<Node>, DirectiveError> {
        let name = cx.name().to_owned();
        let call = json!({
            "kind": "directive",
            "name": name,
            "context": {
                "name": cx.name(),
                "arguments": cx.arguments(),
                "options": options_json(&cx.options()),
                "content": cx.content(),
                "lineno": cx.line(),
                "block_text": cx.block_text(),
                "span": cx.span(),
                "in_substitution": cx.in_substitution(),
            },
        });
        let answer = self.bridge.call(call, Some(cx));
        outcome(answer, &format!("Directive \"{name}\""))
    }
}

struct JsRole {
    bridge: Arc<ParserSide>,
}

impl Role for JsRole {
    fn run(&self, cx: &mut RoleContext<'_>) -> std::result::Result<Vec<Node>, DirectiveError> {
        let call = json!({
            "kind": "role",
            "name": cx.name(),
            "context": {
                "name": cx.name(),
                "text": cx.text(),
                "raw_text": cx.raw_text(),
                "rawsource": cx.rawsource(),
                "options": options_json(&cx.options()),
                "lineno": cx.line(),
                "span": cx.span(),
            },
        });
        let answer = self.bridge.call(call, None);
        outcome(answer, &format!("Role \"{}\"", cx.name()))
    }
}

/// File and URL access through JS functions. The wrapper answers
/// `["ok", latin1]` (the bytes as a Latin-1 string), `["oserror", code,
/// message]` or `["throw"]`.
struct JsLoader {
    files: bool,
    urls: bool,
    bridge: Arc<ParserSide>,
}

impl JsLoader {
    fn read(&self, kind: &str, arg: &str) -> io::Result<Vec<u8>> {
        let answer = self
            .bridge
            .call(json!({"kind": kind, "name": arg}), None)
            .ok_or_else(|| io::Error::other("the loader stopped"))?;
        let items = answer.as_array().cloned().unwrap_or_default();
        let text = |i: usize| items.get(i).and_then(Value::as_str).unwrap_or_default();
        match text(0) {
            "ok" => Ok(text(1).chars().map(|c| c as u32 as u8).collect()),
            "oserror" => {
                let kind = match text(1) {
                    "ENOENT" => io::ErrorKind::NotFound,
                    "EACCES" | "EPERM" => io::ErrorKind::PermissionDenied,
                    "EISDIR" => io::ErrorKind::IsADirectory,
                    _ => io::ErrorKind::Other,
                };
                Err(io::Error::new(kind, text(2).to_owned()))
            }
            _ => Err(io::Error::other("the loader raised an exception")),
        }
    }
}

impl FileLoader for JsLoader {
    fn read_file(&self, path: &str) -> io::Result<Vec<u8>> {
        if !self.files {
            return Err(io::ErrorKind::NotFound.into());
        }
        self.read("file", path)
    }

    fn read_url(&self, url: &str) -> io::Result<Vec<u8>> {
        if !self.urls {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "URL access is not supported",
            ));
        }
        self.read("url", url)
    }
}

// -- options ---------------------------------------------------------------------

/// The parser's options, from `index.js`.
#[derive(serde::Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Options {
    source: Option<String>,
    transforms: bool,
    keep_unknown_directives: bool,
    settings: Option<Settings>,
    /// Directive specs by name.
    directives: Vec<(String, Spec)>,
    roles: Vec<String>,
    root: Option<String>,
    files: bool,
    urls: bool,
}

#[derive(serde::Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Spec {
    required_arguments: usize,
    optional_arguments: usize,
    final_argument_whitespace: bool,
    has_content: bool,
    /// Option names and kinds: a docutils conversion function name, or a
    /// list of choices.
    options: Vec<(String, Value)>,
}

fn option_kind(kind: &Value) -> Result<OptionKind> {
    if let Value::Array(choices) = kind {
        return Ok(OptionKind::Choice(
            choices
                .iter()
                .map(|c| c.as_str().unwrap_or_default().to_owned())
                .collect(),
        ));
    }
    Ok(match kind.as_str().unwrap_or_default() {
        "flag" => OptionKind::Flag,
        "unchanged" => OptionKind::Text,
        "unchanged_required" => OptionKind::RequiredText,
        "path" => OptionKind::Path,
        "uri" => OptionKind::Uri,
        "int" => OptionKind::Int,
        "nonnegative_int" => OptionKind::NonNegativeInt,
        "percentage" => OptionKind::Percentage,
        "length_or_unitless" => OptionKind::Length,
        "length_or_percentage_or_unitless" => OptionKind::LengthOrPercentage,
        "class_option" => OptionKind::Class,
        other => return Err(to_napi(format!("unknown option kind {other:?}"))),
    })
}

/// The parser for `options`, its JS extensions talking through `bridge`.
fn build(options: Options, bridge: &Arc<ParserSide>) -> Result<Parser> {
    let mut parser = Parser::new()
        .transforms(options.transforms)
        .keep_unknown_directives(options.keep_unknown_directives);
    if let Some(source) = options.source {
        parser = parser.source(source);
    }
    if let Some(settings) = options.settings {
        parser = parser.settings(settings);
    }
    for (name, spec) in options.directives {
        let mut out = DirectiveSpec {
            required_arguments: spec.required_arguments,
            optional_arguments: spec.optional_arguments,
            final_argument_whitespace: spec.final_argument_whitespace,
            has_content: spec.has_content,
            options: Vec::new(),
        };
        for (option, kind) in &spec.options {
            out.options.push((option.clone(), option_kind(kind)?));
        }
        let bridge = bridge.clone();
        parser = parser.directive(&name, JsDirective { spec: out, bridge });
    }
    for name in options.roles {
        let bridge = bridge.clone();
        parser = parser.role(&name, JsRole { bridge });
    }
    if let Some(root) = options.root {
        parser = parser.file_loader(lsdocs_rst::DirectoryLoader::new(root));
    } else if options.files || options.urls {
        let bridge = bridge.clone();
        parser = parser.file_loader(JsLoader {
            files: options.files,
            urls: options.urls,
            bridge,
        });
    }
    Ok(parser)
}

// -- exports -----------------------------------------------------------------------

/// Parses `text` with `options` (JSON); JS extensions run through
/// `dispatch`. Returns `[document, messages]` as JSON.
#[napi]
pub fn parse(
    env: Env,
    text: String,
    options: String,
    dispatch: Function<'_, String, String>,
) -> Result<String> {
    let options: Options = from_json(&options)?;
    let (to_js, from_parser) = channel();
    let (to_parser, from_js) = channel();
    let parser_side = Arc::new(ParserSide {
        to_js,
        from_js: Mutex::new(from_js),
    });
    let parser = build(options, &parser_side)?;
    let js_side = Rc::new(JsSide {
        dispatch: dispatch.create_ref()?,
        to_parser,
        from_parser,
    });
    PARSES.with(|p| p.borrow_mut().push(js_side.clone()));
    let _running = Running;
    let done = std::thread::scope(|scope| {
        let worker = parser_side.clone();
        scope.spawn(move || {
            let result = parser.parse_with_messages(&text);
            let _ = worker.to_js.send(ToJs::Done(Box::new(result)));
        });
        drop(parser_side);
        match js_side.serve(&env) {
            Ok(Woken::Done(done)) => Ok(done),
            Ok(Woken::Reply(_)) => Err(to_napi("unexpected reply from the parser")),
            Err(e) => Err(e),
        }
    })?;
    to_json(&*done)
}

/// A request from a directive's context: `parse_content` (a container
/// element), `parse_inline` (text) or `message` (`[level, text]`), as JSON.
/// Returns `{"ok": result}` or `{"error": message}` as JSON.
#[napi]
pub fn request(env: Env, kind: String, payload: String) -> Result<String> {
    let request = match kind.as_str() {
        "parse_content" => Request::ParseContent(from_json(&payload)?),
        "parse_inline" => Request::ParseInline(from_json(&payload)?),
        "message" => {
            let (level, text): (i64, String) = from_json(&payload)?;
            Request::Message(level, text)
        }
        other => return Err(to_napi(format!("unknown request {other:?}"))),
    };
    let Some(js_side) = PARSES.with(|p| p.borrow().last().cloned()) else {
        return Err(to_napi(
            "the directive has returned; its context can no longer parse",
        ));
    };
    js_side
        .to_parser
        .send(ToParser::Request(request))
        .map_err(|_| to_napi("the parser stopped unexpectedly"))?;
    match js_side.serve(&env)? {
        Woken::Reply(reply) => Ok(reply),
        Woken::Done(_) => Err(to_napi(
            "the directive has returned; its context can no longer parse",
        )),
    }
}

/// The pseudo-XML of a node given as JSON.
#[napi]
pub fn pformat(tree: String) -> Result<String> {
    let node: Node = from_json(&tree)?;
    Ok(node.pformat())
}

/// The Docutils XML of an element given as JSON; `options` as
/// `lsdocs_rst::XmlOptions` (JSON).
#[napi]
pub fn to_xml(tree: String, options: String) -> Result<String> {
    let element: Element = from_json(&tree)?;
    let options: XmlOptions = from_json(&options)?;
    Ok(element.to_xml(&options))
}

/// docutils' `make_id`.
#[napi]
pub fn make_id(name: String) -> String {
    lsdocs_rst::make_id(&name)
}

/// docutils' defaults for the settings, as JSON.
#[napi]
pub fn default_settings() -> Result<String> {
    to_json(&Settings::default())
}
