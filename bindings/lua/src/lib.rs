//! Lua bindings for lsdocs-rst: the `lsdocs_rst` Lua module.
//!
//! ```lua
//! local rst = require("lsdocs_rst")
//! local doc, messages = rst.parse("Hello, *world*.\n", { transforms = true })
//! print(rst.pformat(doc))
//! ```
//!
//! Trees cross as plain Lua tables in the form of the `serde` feature of
//! `lsdocs-rst`: `{ kind, attributes, children, span }` for an element and
//! `{ text, span }` for text. A missing value (`span` of a node without a
//! source position, say) is `nil`, and arrays have no metatable. An
//! attribute value of Python's `None` (in `pending` details only) is
//! `lsdocs_rst.null`.
//!
//! Directives, roles and file loaders can be Lua functions. The parser's
//! extensions must be `Send + Sync`, and Lua values are neither (mlua's
//! `send` feature does not build in module mode), so an extension holds
//! only the registry key of its function and finds the Lua state through a
//! thread-local that `parse` sets: the parse runs on the calling thread,
//! which is where every extension then runs.
//!
//! Built with the `module` feature, this is a loadable module for an
//! external Lua 5.4 or LuaJIT. Without it (the default), the crate embeds
//! a vendored Lua 5.4 and [`open`] is what the tests call.

use std::cell::RefCell;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};

use lsdocs_rst::{
    Directive, DirectiveContext, DirectiveError, DirectiveSpec, Element, FileLoader, Node,
    OptionKind, OptionValue, Parser, Role, RoleContext, Settings, Span, XmlOptions,
};
use mlua::{
    Error, Function, Lua, LuaSerdeExt, MultiValue, Result, Table, UserData, UserDataMethods, Value,
    WeakLua, serde::SerializeOptions,
};

/// How values are turned into Lua: `nil` for `None` (a missing `span`, say),
/// and arrays without the array metatable, so trees are plain tables. An
/// attribute whose value is Python's `None` (only in `pending` details)
/// would vanish as `nil`, so it is `lsdocs_rst.null` (mlua's null
/// light userdata) instead.
fn options() -> SerializeOptions {
    SerializeOptions::new()
        .serialize_none_to_null(false)
        .serialize_unit_to_null(true)
        .set_array_metatable(false)
}

fn to_lua<T: serde::Serialize + ?Sized>(lua: &Lua, value: &T) -> Result<Value> {
    lua.to_value_with(value, options())
}

/// A tree (node, element, or list of them) from its table form.
///
/// An empty table is an empty array or an empty map; without the array
/// metatable Lua cannot say which, and the tree's untagged `Node` needs to
/// know. So the table goes through JSON first, where an empty `children`
/// or attribute value becomes an empty list (no attribute value is a map),
/// and a string in `children` becomes a text node.
fn tree_from_lua<T: serde::de::DeserializeOwned>(lua: &Lua, value: Value) -> Result<T> {
    let mut json: serde_json::Value = lua.from_value(value)?;
    empty_lists(&mut json);
    serde_json::from_value(json).map_err(mlua::Error::external)
}

fn is_empty_object(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(serde_json::Map::is_empty)
}

/// Strings in a list of nodes, as text nodes.
fn strings_as_text(items: &mut [serde_json::Value]) {
    for item in items {
        if let serde_json::Value::String(text) = item {
            *item = serde_json::json!({ "text": std::mem::take(text) });
        }
    }
}

fn empty_lists(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => items.iter_mut().for_each(empty_lists),
        serde_json::Value::Object(node) => {
            if let Some(children) = node.get_mut("children") {
                if is_empty_object(children) {
                    *children = serde_json::Value::Array(Vec::new());
                }
                if let serde_json::Value::Array(items) = children {
                    strings_as_text(items);
                }
                empty_lists(children);
            }
            if let Some(serde_json::Value::Object(attributes)) = node.get_mut("attributes") {
                for v in attributes.values_mut() {
                    if is_empty_object(v) {
                        *v = serde_json::Value::Array(Vec::new());
                    }
                }
            }
        }
        _ => {}
    }
}

/// Nodes from what a directive or role returned: a list of nodes in table
/// form, where a string stands for a text node.
fn nodes_from_lua(lua: &Lua, value: Value) -> Result<Vec<Node>> {
    let mut json: serde_json::Value = lua.from_value(value)?;
    if is_empty_object(&json) {
        json = serde_json::Value::Array(Vec::new());
    }
    if let serde_json::Value::Array(items) = &mut json {
        strings_as_text(items);
    }
    empty_lists(&mut json);
    serde_json::from_value(json).map_err(|e| Error::runtime(format!("invalid nodes: {e}")))
}

// -- extensions --------------------------------------------------------------

/// The parse running on this thread: its Lua state, and the errors its
/// extensions raised (the first is raised again once the parse is done).
/// `mlua::Error` is not `Send` either, so they are kept here, not in the
/// extensions.
struct Frame {
    lua: WeakLua,
    raised: Vec<Error>,
}

thread_local! {
    /// The innermost parse on this thread (a Lua extension can parse too).
    static STATE: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` as a parse in `lua`: `f`'s result, and the first error its
/// extensions raised.
fn with_state<T>(lua: &Lua, f: impl FnOnce() -> T) -> (T, Option<Error>) {
    STATE.with(|s| {
        s.borrow_mut().push(Frame {
            lua: lua.weak(),
            raised: Vec::new(),
        });
    });
    let out = f();
    let frame = STATE.with(|s| s.borrow_mut().pop());
    (out, frame.and_then(|f| f.raised.into_iter().next()))
}

fn current_state() -> Result<Lua> {
    STATE
        .with(|s| s.borrow().last().and_then(|f| f.lua.try_upgrade()))
        .ok_or_else(|| Error::runtime("no Lua state on this thread: extensions run from parse"))
}

/// Keeps an error an extension raised, to raise after the parse.
fn keep(err: Error) {
    STATE.with(|s| {
        if let Some(frame) = s.borrow_mut().last_mut() {
            frame.raised.push(err);
        }
    });
}

/// Stores `func` in the registry under a new key.
fn register(lua: &Lua, func: Function) -> Result<String> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let key = format!(
        "lsdocs_rst.extension.{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    lua.set_named_registry_value(&key, func)?;
    Ok(key)
}

fn span_to_lua(lua: &Lua, span: Option<Span>) -> Result<Value> {
    to_lua(lua, &span)
}

/// An option value: `true` for a flag, else its text, number or list.
fn option_to_lua(lua: &Lua, value: &OptionValue) -> Result<Value> {
    Ok(match value {
        OptionValue::Text(s) => Value::String(lua.create_string(s)?),
        OptionValue::Int(n) => Value::Integer(*n),
        OptionValue::List(l) => {
            Value::Table(lua.create_sequence_from(l.iter().map(String::as_str))?)
        }
        // A flag, and any kind added later.
        _ => Value::Boolean(true),
    })
}

fn options_to_lua(lua: &Lua, options: &[(String, OptionValue)]) -> Result<Table> {
    let table = lua.create_table()?;
    for (name, value) in options {
        table.set(name.as_str(), option_to_lua(lua, value)?)?;
    }
    Ok(table)
}

/// What an extension's function returned, for the parser.
type Outcome = std::result::Result<Vec<Node>, DirectiveError>;

/// What a directive or role function returned: nodes, or `nil, message[,
/// level]` for a problem reported in the document (level 3, error, unless
/// given).
fn outcome(lua: &Lua, values: MultiValue) -> Result<Outcome> {
    let mut values = values.into_iter();
    match values.next() {
        None | Some(Value::Nil) => {
            let message = match values.next() {
                Some(Value::String(s)) => s.to_str()?.to_owned(),
                Some(Value::Nil) | None => return Ok(Ok(Vec::new())),
                Some(other) => {
                    return Err(Error::runtime(format!(
                        "expected a message after nil, got a {}",
                        other.type_name()
                    )));
                }
            };
            let level = match values.next() {
                Some(Value::Integer(n)) => n,
                #[allow(clippy::cast_possible_truncation)]
                Some(Value::Number(n)) => n as i64,
                _ => 3,
            };
            Ok(Err(DirectiveError { level, message }))
        }
        Some(nodes) => Ok(Ok(nodes_from_lua(lua, nodes)?)),
    }
}

/// A directive whose `run` is a Lua function.
struct LuaDirective {
    spec: DirectiveSpec,
    key: String,
}

impl LuaDirective {
    fn call(&self, cx: &mut DirectiveContext<'_>) -> Result<Outcome> {
        let lua = current_state()?;
        let run: Function = lua.named_registry_value(&self.key)?;
        let ctx = lua.create_table()?;
        ctx.set("name", cx.name())?;
        ctx.set(
            "arguments",
            lua.create_sequence_from(cx.arguments().iter().map(String::as_str))?,
        )?;
        ctx.set("options", options_to_lua(&lua, &cx.options())?)?;
        ctx.set("content", lua.create_sequence_from(cx.content())?)?;
        ctx.set("lineno", cx.line())?;
        ctx.set("block_text", cx.block_text())?;
        ctx.set("span", span_to_lua(&lua, cx.span())?)?;
        ctx.set("in_substitution", cx.in_substitution())?;
        let cx = RefCell::new(cx);
        let result = lua.scope(|scope| {
            // Methods (`ctx:parse_inline(text)`): the first argument is ctx.
            ctx.set(
                "parse_content",
                scope.create_function(|lua, (_, container): (Value, Value)| {
                    let container: Element = tree_from_lua(lua, container)?;
                    let element = cx.borrow_mut().parse_content(container);
                    to_lua(lua, &element)
                })?,
            )?;
            ctx.set(
                "parse_inline",
                scope.create_function(|lua, (_, text): (Value, String)| {
                    let (nodes, messages) = cx.borrow_mut().parse_inline(&text);
                    Ok((to_lua(lua, &nodes)?, to_lua(lua, &messages)?))
                })?,
            )?;
            ctx.set(
                "message",
                scope.create_function(|lua, (_, level, text): (Value, i64, String)| {
                    let element = cx.borrow_mut().message(level, &text);
                    to_lua(lua, &element)
                })?,
            )?;
            let values: MultiValue = run.call(ctx.clone())?;
            outcome(&lua, values)
        });
        // Once `run` returns, the methods no longer work (the scope has
        // ended): calling them is an error saying so.
        result
    }
}

impl Directive for LuaDirective {
    fn spec(&self) -> DirectiveSpec {
        self.spec.clone()
    }

    fn run(&self, cx: &mut DirectiveContext<'_>) -> Outcome {
        let name = cx.name().to_owned();
        self.call(cx).unwrap_or_else(|err| {
            keep(err);
            Err(DirectiveError::error(format!(
                "Directive \"{name}\" raised an error."
            )))
        })
    }
}

/// An option kind by its docutils conversion function's name, or a list
/// of choices.
fn option_kind(value: Value) -> Result<OptionKind> {
    if let Value::Table(choices) = value {
        return Ok(OptionKind::Choice(
            choices.sequence_values::<String>().collect::<Result<_>>()?,
        ));
    }
    let Value::String(name) = value else {
        return Err(Error::runtime(format!(
            "an option kind is a name or a list of choices, not a {}",
            value.type_name()
        )));
    };
    Ok(match &*name.to_str()? {
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
        other => return Err(Error::runtime(format!("unknown option kind {other:?}"))),
    })
}

/// A directive from its table: `run` and what it accepts.
fn lua_directive(lua: &Lua, entry: &Table) -> Result<LuaDirective> {
    let mut spec = DirectiveSpec::default();
    if let Some(n) = entry.get::<Option<usize>>("required_arguments")? {
        spec.required_arguments = n;
    }
    if let Some(n) = entry.get::<Option<usize>>("optional_arguments")? {
        spec.optional_arguments = n;
    }
    spec.final_argument_whitespace = entry
        .get::<Option<bool>>("final_argument_whitespace")?
        .unwrap_or(false);
    spec.has_content = entry.get::<Option<bool>>("has_content")?.unwrap_or(false);
    if let Some(options) = entry.get::<Option<Table>>("options")? {
        for pair in options.pairs::<String, Value>() {
            let (name, kind) = pair?;
            spec.options.push((name, option_kind(kind)?));
        }
        // A table's order is unspecified; keep the spec stable.
        spec.options.sort_by(|a, b| a.0.cmp(&b.0));
    }
    let run: Function = entry.get("run")?;
    Ok(LuaDirective {
        spec,
        key: register(lua, run)?,
    })
}

/// A role whose function is Lua.
struct LuaRole {
    key: String,
}

impl LuaRole {
    fn call(&self, cx: &RoleContext<'_>) -> Result<Outcome> {
        let lua = current_state()?;
        let run: Function = lua.named_registry_value(&self.key)?;
        let ctx = lua.create_table()?;
        ctx.set("name", cx.name())?;
        ctx.set("text", cx.text())?;
        ctx.set("raw_text", cx.raw_text())?;
        ctx.set("rawsource", cx.rawsource())?;
        ctx.set("options", options_to_lua(&lua, &cx.options())?)?;
        ctx.set("lineno", cx.line())?;
        ctx.set("span", span_to_lua(&lua, cx.span())?)?;
        let values: MultiValue = run.call(ctx)?;
        outcome(&lua, values)
    }
}

impl Role for LuaRole {
    fn run(&self, cx: &mut RoleContext<'_>) -> Outcome {
        self.call(cx).unwrap_or_else(|err| {
            keep(err);
            Err(DirectiveError::error(format!(
                "Role \"{}\" raised an error.",
                cx.name()
            )))
        })
    }
}

/// File and URL access through Lua functions returning the contents, or
/// `nil, message[, kind]` for a failed read reported in the document
/// (`kind`: "not_found", "permission_denied", "is_a_directory"). An error
/// raised is raised again after the parse.
struct LuaLoader {
    files: Option<String>,
    urls: Option<String>,
}

impl LuaLoader {
    fn call(key: &str, arg: &str) -> Result<io::Result<Vec<u8>>> {
        let lua = current_state()?;
        let read: Function = lua.named_registry_value(key)?;
        let (contents, message, kind): (Value, Option<String>, Option<String>) = read.call(arg)?;
        Ok(match contents {
            Value::String(s) => Ok(s.as_bytes().to_vec()),
            Value::Nil => {
                let kind = match kind.as_deref() {
                    Some("not_found") => io::ErrorKind::NotFound,
                    Some("permission_denied") => io::ErrorKind::PermissionDenied,
                    Some("is_a_directory") => io::ErrorKind::IsADirectory,
                    _ => io::ErrorKind::Other,
                };
                Err(io::Error::new(
                    kind,
                    message.unwrap_or_else(|| "cannot read".to_owned()),
                ))
            }
            other => {
                return Err(Error::runtime(format!(
                    "a loader returns a string or nil, not a {}",
                    other.type_name()
                )));
            }
        })
    }

    fn read(&self, key: Option<&String>, arg: &str) -> io::Result<Vec<u8>> {
        let Some(key) = key else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "URL access is not supported",
            ));
        };
        Self::call(key, arg).unwrap_or_else(|err| {
            let message = err.to_string();
            keep(err);
            Err(io::Error::other(message))
        })
    }
}

impl FileLoader for LuaLoader {
    fn read_file(&self, path: &str) -> io::Result<Vec<u8>> {
        self.read(self.files.as_ref(), path)
    }

    fn read_url(&self, url: &str) -> io::Result<Vec<u8>> {
        self.read(self.urls.as_ref(), url)
    }
}

// -- the parser -------------------------------------------------------------------

/// A field of an options table, if the table and the field are given.
fn field<T: mlua::FromLua>(opts: Option<&Table>, name: &str) -> Result<Option<T>> {
    match opts {
        Some(opts) => opts.get(name),
        None => Ok(None),
    }
}

/// A parser from an options table: `source`, `transforms`,
/// `keep_unknown_directives`, `settings` (docutils setting names), `root`
/// (a directory `include` and friends read below) or `files`/`urls`
/// (loader functions), `directives` and `roles`.
fn make_parser(lua: &Lua, opts: Option<Table>) -> Result<LuaParser> {
    let opts = opts.as_ref();
    let mut parser = Parser::new()
        .transforms(field(opts, "transforms")?.unwrap_or(false))
        .keep_unknown_directives(field(opts, "keep_unknown_directives")?.unwrap_or(false));
    if let Some(source) = field::<String>(opts, "source")? {
        parser = parser.source(source);
    }
    if let Some(settings) = field::<Value>(opts, "settings")? {
        let settings: Settings = lua.from_value(settings)?;
        parser = parser.settings(settings);
    }
    let files = field::<Function>(opts, "files")?;
    let urls = field::<Function>(opts, "urls")?;
    match (
        field::<String>(opts, "root")?,
        files.is_some() || urls.is_some(),
    ) {
        (Some(_), true) => {
            return Err(Error::runtime(
                "give a root directory or loader functions, not both",
            ));
        }
        (Some(root), false) => {
            parser = parser.file_loader(lsdocs_rst::DirectoryLoader::new(root));
        }
        (None, true) => {
            parser = parser.file_loader(LuaLoader {
                files: files.map(|f| register(lua, f)).transpose()?,
                urls: urls.map(|f| register(lua, f)).transpose()?,
            });
        }
        (None, false) => {}
    }
    if let Some(directives) = field::<Table>(opts, "directives")? {
        for pair in directives.pairs::<String, Table>() {
            let (name, entry) = pair?;
            parser = parser.directive(&name, lua_directive(lua, &entry)?);
        }
    }
    if let Some(roles) = field::<Table>(opts, "roles")? {
        for pair in roles.pairs::<String, Function>() {
            let (name, run) = pair?;
            let role = LuaRole {
                key: register(lua, run)?,
            };
            parser = parser.role(&name, role);
        }
    }
    Ok(LuaParser(parser))
}

/// `document, messages` for `text`. An error a Lua extension raised is
/// raised again here.
fn parse_with(lua: &Lua, parser: &LuaParser, text: &str) -> Result<MultiValue> {
    let ((document, messages), raised) = with_state(lua, || parser.0.parse_with_messages(text));
    if let Some(err) = raised {
        return Err(err);
    }
    Ok(MultiValue::from_vec(vec![
        to_lua(lua, &document)?,
        to_lua(lua, &messages)?,
    ]))
}

/// A configured parser, made by `lsdocs_rst.parser(options)`.
struct LuaParser(Parser);

impl UserData for LuaParser {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("parse", |lua, this, text: String| {
            parse_with(lua, this, &text)
        });
        methods.add_method("settings", |lua, this, ()| {
            to_lua(lua, this.0.current_settings())
        });
    }
}

/// The module table.
///
/// # Errors
///
/// If Lua cannot allocate the table or its functions.
pub fn open(lua: &Lua) -> Result<Table> {
    let m = lua.create_table()?;
    m.set("version", env!("CARGO_PKG_VERSION"))?;
    m.set("null", lua.null())?;
    m.set(
        "parser",
        lua.create_function(|lua, opts: Option<Table>| make_parser(lua, opts))?,
    )?;
    m.set(
        "parse",
        lua.create_function(|lua, (text, opts): (String, Option<Table>)| {
            parse_with(lua, &make_parser(lua, opts)?, &text)
        })?,
    )?;
    m.set(
        "pformat",
        lua.create_function(|lua, tree: Value| {
            let node: Node = tree_from_lua(lua, tree)?;
            Ok(node.pformat())
        })?,
    )?;
    m.set(
        "to_xml",
        lua.create_function(|lua, (tree, opts): (Value, Option<Value>)| {
            let element: Element = tree_from_lua(lua, tree)?;
            let opts: XmlOptions = match opts {
                Some(opts) => lua.from_value(opts)?,
                None => XmlOptions::default(),
            };
            Ok(element.to_xml(&opts))
        })?,
    )?;
    m.set(
        "make_id",
        lua.create_function(|_, name: String| Ok(lsdocs_rst::make_id(&name)))?,
    )?;
    m.set(
        "default_settings",
        lua.create_function(|lua, ()| to_lua(lua, &Settings::default()))?,
    )?;
    Ok(m)
}

/// The entry point an external interpreter calls (`luaopen_lsdocs_rst`).
#[cfg(feature = "module")]
#[mlua::lua_module(name = "lsdocs_rst")]
fn luaopen(lua: &Lua) -> Result<Table> {
    open(lua)
}
